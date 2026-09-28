//! 服务端黑盒测试：拉起真实的 secunzip-server 进程，用原始 HTTP/1.1 请求打接口。
//!
//! 刻意不引入 HTTP 客户端依赖，只用 std 的 TcpStream 手写请求，避免为了测试扩大供应链表面积。
//! 每个测试独立占用一个临时目录和一个空闲端口，可并行执行。

use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_secunzip-server");
const ROOT_URL: &str = "sqlite:secunzip.db?mode=rwc";

/// 测试用运行时：sqlx 的连接池需要挂在某个 runtime 上，所有测试共用一个多线程 runtime。
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("无法创建 tokio runtime"))
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("secunzip-it-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("无法创建临时目录");
    dir
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("无法探测空闲端口")
        .local_addr()
        .unwrap()
        .port()
}

fn pool_for(db: &Path) -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .filename(db)
        .create_if_missing(true);
    runtime().block_on(async { SqlitePoolOptions::new().connect_with(opts).await.unwrap() })
}

/// 带超时地跑一次 secunzip-server 并收集 stderr。
///
/// 直接用 `Command::output()` 时，只要服务端因为任何原因没有退出，测试就会永久挂起，
/// 而且不打印任何东西——很难定位。这里统一给 30 秒上限，超时就杀掉并报错。
fn run_bin(cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> (bool, String) {
    let mut cmd = Command::new(BIN);
    cmd.current_dir(cwd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().expect("无法启动 secunzip-server");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait().expect("无法查询子进程状态") {
            Some(status) => {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                return (status.success(), stderr);
            }
            None => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("secunzip-server 在 30 秒内没有退出（参数: {:?}）", args);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// 一个跑起来的服务端进程；Drop 时杀掉并清理临时目录。
struct Server {
    child: Option<Child>,
    port: u16,
    dir: PathBuf,
}

impl Server {
    fn start(name: &str) -> Self {
        Self::start_env(name, ROOT_URL, &[])
    }

    fn start_env(name: &str, db_url: &str, envs: &[(&str, &str)]) -> Self {
        Self::start_in_dir(&temp_dir(name), db_url, envs)
    }

    /// 在一个已存在的目录里启动，用于需要在服务端启动前预置数据库文件的场景。
    fn start_in_dir(dir: &Path, db_url: &str, envs: &[(&str, &str)]) -> Self {
        let port = free_port();
        let log = std::fs::File::create(dir.join("server.log")).unwrap();

        let mut cmd = Command::new(BIN);
        cmd.current_dir(dir)
            .env("SECUNZIP_PORT", port.to_string())
            .env("DATABASE_URL", db_url)
            .stdout(Stdio::from(log))
            .stderr(Stdio::null());
        for (k, v) in envs {
            cmd.env(k, v);
        }

        let child = cmd.spawn().expect("无法启动 secunzip-server");
        let mut s = Server {
            child: Some(child),
            port,
            dir: dir.to_path_buf(),
        };
        s.wait_ready();
        s
    }

    fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                return;
            }
            if let Some(c) = &mut self.child {
                if let Ok(Some(st)) = c.try_wait() {
                    let log =
                        std::fs::read_to_string(self.dir.join("server.log")).unwrap_or_default();
                    panic!("服务端提前退出（{:?}），日志：\n{}", st, log);
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("服务端在 20 秒内没有开始监听端口 {}", self.port);
    }

    fn db(&self) -> PathBuf {
        self.dir.join("secunzip.db")
    }

    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    fn post(&self, path: &str, body: Value) -> Value {
        post_json(self.port, path, body)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 把 HTTP 响应切成 body：兼容 Content-Length 与 chunked 两种情形。
fn body_of(raw: &[u8]) -> Vec<u8> {
    let pos = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("响应缺少头部终止符");
    let head = String::from_utf8_lossy(&raw[..pos]).to_ascii_lowercase();
    let body = &raw[pos + 4..];
    if !head.contains("transfer-encoding: chunked") {
        return body.to_vec();
    }
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(p) = rest.windows(2).position(|w| w == b"\r\n") {
        let size =
            usize::from_str_radix(String::from_utf8_lossy(&rest[..p]).trim(), 16).unwrap_or(0);
        if size == 0 || rest.len() < p + 2 + size {
            break;
        }
        out.extend_from_slice(&rest[p + 2..p + 2 + size]);
        rest = &rest[(p + 2 + size + 2).min(rest.len())..];
    }
    out
}

fn post_json(port: u16, path: &str, body: Value) -> Value {
    let payload = body.to_string();
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("连接失败");
    s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        path,
        payload.len(),
        payload
    );
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let body = body_of(&raw);
    serde_json::from_slice(&body).unwrap_or_else(|e| {
        panic!(
            "{} 的响应不是 JSON: {}\n原文: {}",
            path,
            e,
            String::from_utf8_lossy(&raw)
        )
    })
}

/// 发一个 GET 请求，返回 (HTTP 状态码, 解析后的 JSON body)。
///
/// 与 post_json 一样手写原始 HTTP；因为健康检查要断言的正是状态行，
/// 而 post_json 只把状态行丢掉、只返回 body，所以这里单列一个函数。
fn get_json(port: u16, path: &str) -> (u16, Value) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("连接失败");
    s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        path
    );
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let head = String::from_utf8_lossy(&raw).to_string();
    let status: u16 = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("{} 的响应缺少状态行\n原文: {}", path, head));
    let body = body_of(&raw);
    let json = serde_json::from_slice(&body)
        .unwrap_or_else(|e| panic!("{} 的响应不是 JSON: {}\n原文: {}", path, e, head));
    (status, json)
}

fn register(port: u16, app: &str, secret: &str, content_key: &str, allow_temp: bool) {
    let r = post_json(
        port,
        "/api/register",
        json!({ "app_id": app, "secret": secret, "content_key": content_key, "allow_temp": allow_temp }),
    );
    assert_eq!(r["success"], json!(true), "注册失败: {}", r);
}

/// 带 ip_whitelist 的注册，返回原始响应以便断言失败情形。
/// 单独一个函数：`register` 必须继续只发旧字段，用来验证省略该字段的向后兼容。
fn register_with_ip_whitelist(
    port: u16,
    app: &str,
    secret: &str,
    content_key: &str,
    allow_temp: bool,
    ip_whitelist: Value,
) -> Value {
    post_json(
        port,
        "/api/register",
        json!({
            "app_id": app,
            "secret": secret,
            "content_key": content_key,
            "allow_temp": allow_temp,
            "ip_whitelist": ip_whitelist,
        }),
    )
}

fn text(v: &Value, field: &str) -> String {
    v[field].as_str().unwrap_or_default().to_string()
}

// ===== 健康检查 =====

#[test]
fn test_healthz_returns_ok() {
    let s = Server::start("healthz");
    let (status, body) = get_json(s.port, "/healthz");
    assert_eq!(status, 200, "健康检查应返回 200，实际 body: {}", body);
    assert_eq!(body, json!({ "status": "ok" }));
}

// ===== 注册与防覆盖 =====

#[test]
fn test_admin_secret_returns_content_key() {
    let s = Server::start("admin-key");
    register(s.port, "app-a", "secret-a", "KEY-A", false);

    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-a", "secret": "secret-a" }),
    );
    assert_eq!(r["success"], json!(true));
    assert_eq!(text(&r, "key"), "KEY-A");
}

#[test]
fn test_register_rejects_overwrite_by_other_secret() {
    let s = Server::start("anti-overwrite");
    register(s.port, "app-a", "secret-a", "KEY-A", false);

    // 攻击者知道 app_id（就是文件 MD5，人手一份文件就能算出来），但不知道 secret
    let r = s.post(
        "/api/register",
        json!({ "app_id": "app-a", "secret": "attacker", "content_key": "KEY-EVIL" }),
    );
    assert_eq!(r["success"], json!(false), "伪装注册必须被拒绝");
    assert!(
        text(&r, "message").contains("拒绝覆盖"),
        "错误信息应说明原因: {}",
        r
    );

    // 关键断言：原来的内容密钥必须原封不动，文件不能被弄砖
    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-a", "secret": "secret-a" }),
    );
    assert_eq!(text(&r, "key"), "KEY-A", "内容密钥被覆盖了");

    // 攻击者用自己的 secret 也拿不到东西
    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-a", "secret": "attacker" }),
    );
    assert_eq!(r["success"], json!(false));
}

#[test]
fn test_register_allows_update_with_same_secret() {
    let s = Server::start("re-register");
    register(s.port, "app-a", "secret-a", "KEY-OLD", false);
    register(s.port, "app-a", "secret-a", "KEY-NEW", true);

    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-a", "secret": "secret-a" }),
    );
    assert_eq!(text(&r, "key"), "KEY-NEW");
}

// ===== 授权与取密钥 =====

#[test]
fn test_key_requires_grant() {
    let s = Server::start("no-grant");
    register(s.port, "app-a", "secret-a", "KEY-A", true);

    let r = s.post("/api/key", json!({ "app_id": "app-a", "user_id": "u1" }));
    assert_eq!(r["success"], json!(false));
    assert!(r.get("key").is_none(), "拒绝时不能带 key 字段");
    assert!(text(&r, "message").contains("未授权"));
}

#[test]
fn test_grant_then_revoke_blocks_key() {
    let s = Server::start("grant-revoke");
    register(s.port, "app-a", "secret-a", "KEY-A", true);

    let r = s.post(
        "/api/grant",
        json!({ "app_id": "app-a", "secret": "secret-a", "user_id": "u1" }),
    );
    assert_eq!(r["success"], json!(true));

    let r = s.post("/api/key", json!({ "app_id": "app-a", "user_id": "u1" }));
    assert_eq!(text(&r, "key"), "KEY-A");

    let r = s.post(
        "/api/revoke",
        json!({ "app_id": "app-a", "secret": "secret-a", "user_id": "u1" }),
    );
    assert_eq!(r["success"], json!(true));

    let r = s.post("/api/key", json!({ "app_id": "app-a", "user_id": "u1" }));
    assert_eq!(r["success"], json!(false), "吊销后必须取不到密钥");
}

#[test]
fn test_wrong_secret_cannot_grant_or_revoke() {
    let s = Server::start("wrong-secret");
    register(s.port, "app-a", "secret-a", "KEY-A", true);

    let r = s.post(
        "/api/grant",
        json!({ "app_id": "app-a", "secret": "bad", "user_id": "u1" }),
    );
    assert_eq!(r["success"], json!(false));

    // 用错误密钥授权不生效，用户仍然拿不到密钥
    let r = s.post("/api/key", json!({ "app_id": "app-a", "user_id": "u1" }));
    assert_eq!(r["success"], json!(false));

    // 用错误密钥也不能吊销别人的授权
    let r = s.post(
        "/api/grant",
        json!({ "app_id": "app-a", "secret": "secret-a", "user_id": "u1" }),
    );
    assert_eq!(r["success"], json!(true));
    let r = s.post(
        "/api/revoke",
        json!({ "app_id": "app-a", "secret": "bad", "user_id": "u1" }),
    );
    assert_eq!(r["success"], json!(false));
    let r = s.post("/api/key", json!({ "app_id": "app-a", "user_id": "u1" }));
    assert_eq!(text(&r, "key"), "KEY-A", "错误密钥的吊销不应生效");
}

#[test]
fn test_expired_grant_rejected_and_pruned() {
    let s = Server::start("expired");
    register(s.port, "app-a", "secret-a", "KEY-A", true);
    // 过期时间写成很久以前
    let r = s.post(
        "/api/grant",
        json!({ "app_id": "app-a", "secret": "secret-a", "user_id": "u1", "expires_at": "20200101" }),
    );
    assert_eq!(r["success"], json!(true));

    let r = s.post("/api/key", json!({ "app_id": "app-a", "user_id": "u1" }));
    assert_eq!(r["success"], json!(false));
    assert!(text(&r, "message").contains("过期"), "应提示过期: {}", r);

    // 过期记录应被顺手删除
    let pool = pool_for(&s.db());
    let n: i64 = runtime().block_on(async {
        sqlx::query("SELECT COUNT(*) AS n FROM grants WHERE app_id = 'app-a' AND user_id = 'u1'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("n")
    });
    runtime().block_on(pool.close());
    assert_eq!(n, 0, "过期授权应被清理");
}

#[test]
fn test_permanent_grant_never_expires() {
    let s = Server::start("permanent");
    register(s.port, "app-a", "secret-a", "KEY-A", true);
    let r = s.post(
        "/api/grant",
        json!({ "app_id": "app-a", "secret": "secret-a", "user_id": "u1", "expires_at": "永久" }),
    );
    assert_eq!(r["success"], json!(true));
    let r = s.post("/api/key", json!({ "app_id": "app-a", "user_id": "u1" }));
    assert_eq!(text(&r, "key"), "KEY-A");
}

#[test]
fn test_key_for_unknown_app_fails() {
    let s = Server::start("no-app");
    // 先注册一个别的应用，保证库里有东西
    register(s.port, "app-a", "secret-a", "KEY-A", false);
    let r = s.post(
        "/api/key",
        json!({ "app_id": "missing", "secret": "whatever" }),
    );
    assert_eq!(r["success"], json!(false));
}

// ===== 来源 IP 白名单 =====

#[test]
fn test_ip_whitelist_allows_matching_source() {
    let s = Server::start("ip-allow");
    // 测试请求从 127.0.0.1 发起，CIDR 与单机两种写法都应命中
    let r = register_with_ip_whitelist(
        s.port,
        "app-cidr",
        "secret-a",
        "KEY-A",
        true,
        json!(["127.0.0.0/8"]),
    );
    assert_eq!(r["success"], json!(true), "注册失败: {}", r);
    let r = register_with_ip_whitelist(
        s.port,
        "app-host",
        "secret-b",
        "KEY-B",
        true,
        json!(["127.0.0.1", "10.0.0.0/8"]),
    );
    assert_eq!(r["success"], json!(true), "注册失败: {}", r);

    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-cidr", "secret": "secret-a" }),
    );
    assert_eq!(text(&r, "key"), "KEY-A", "命中的来源 IP 应能取钥: {}", r);

    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-host", "secret": "secret-b" }),
    );
    assert_eq!(text(&r, "key"), "KEY-B", "命中的来源 IP 应能取钥: {}", r);
}

#[test]
fn test_ip_whitelist_blocks_non_matching_source() {
    let s = Server::start("ip-deny");
    let r = register_with_ip_whitelist(
        s.port,
        "app-a",
        "secret-a",
        "KEY-A",
        true,
        json!(["10.0.0.0/8"]),
    );
    assert_eq!(r["success"], json!(true), "注册失败: {}", r);

    // 管理员路径同样受白名单约束
    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-a", "secret": "secret-a" }),
    );
    assert_eq!(r["success"], json!(false), "非白名单来源必须被拒: {}", r);
    assert!(r.get("key").is_none(), "拒绝时不能带 key 字段: {}", r);
    assert!(text(&r, "message").contains("白名单"), "应说明原因: {}", r);

    // 已授权用户路径同样被拒
    let r = s.post(
        "/api/grant",
        json!({ "app_id": "app-a", "secret": "secret-a", "user_id": "u1" }),
    );
    assert_eq!(r["success"], json!(true));
    let r = s.post("/api/key", json!({ "app_id": "app-a", "user_id": "u1" }));
    assert_eq!(r["success"], json!(false), "非白名单来源必须被拒: {}", r);
    assert!(r.get("key").is_none(), "拒绝时不能带 key 字段: {}", r);
    assert!(text(&r, "message").contains("白名单"), "应说明原因: {}", r);

    // 拒绝必须留下可诊断的审计日志
    let r = s.post(
        "/api/logs",
        json!({ "app_id": "app-a", "secret": "secret-a" }),
    );
    let logs = r["logs"].as_array().unwrap();
    let denied: Vec<&Value> = logs
        .iter()
        .filter(|l| text(l, "action") == "key" && l["success"] == json!(false))
        .collect();
    assert!(denied.len() >= 2, "被拒的取钥应计入审计: {}", r);
    assert!(
        denied.iter().any(|l| text(l, "details").contains("白名单")),
        "审计详情应写明 IP 白名单原因: {}",
        r
    );
}

#[test]
fn test_register_without_ip_whitelist_keeps_unrestricted() {
    let s = Server::start("ip-unset");
    // 完全省略该字段：与旧版行为一致，不限制来源
    register(s.port, "app-a", "secret-a", "KEY-A", false);
    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-a", "secret": "secret-a" }),
    );
    assert_eq!(text(&r, "key"), "KEY-A", "省略白名单字段应不限制: {}", r);

    // 显式空数组等价于不限制
    let r = register_with_ip_whitelist(s.port, "app-b", "secret-b", "KEY-B", false, json!([]));
    assert_eq!(r["success"], json!(true), "空数组应被接受: {}", r);
    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-b", "secret": "secret-b" }),
    );
    assert_eq!(text(&r, "key"), "KEY-B", "空数组应不限制: {}", r);
}

#[test]
fn test_register_rejects_malformed_ip_whitelist() {
    let s = Server::start("ip-bad");
    // 前缀越界、非法文本、IPv6、缺失前缀都必须在注册时拒绝
    for bad in ["10.0.0.0/33", "not-an-ip", "2001:db8::1", "10.0.0.0/"] {
        let r = register_with_ip_whitelist(
            s.port,
            "app-bad",
            "secret-bad",
            "KEY-BAD",
            false,
            json!([bad]),
        );
        assert_eq!(
            r["success"],
            json!(false),
            "非法条目 {:?} 应被拒: {}",
            bad,
            r
        );
        assert!(text(&r, "message").contains("非法"), "应说明原因: {}", r);
    }

    // 被拒的注册不得写库：换一个 secret 仍应能注册成功（否则会被防覆盖规则挡住）
    let r = post_json(
        s.port,
        "/api/register",
        json!({ "app_id": "app-bad", "secret": "secret-ok", "content_key": "KEY-OK" }),
    );
    assert_eq!(r["success"], json!(true), "非法条目不应写库: {}", r);
}

// ===== 临时申请审批 =====

#[test]
fn test_request_rejected_when_temp_disabled() {
    let s = Server::start("temp-off");
    register(s.port, "app-a", "secret-a", "KEY-A", false);
    let r = s.post(
        "/api/request",
        json!({ "app_id": "app-a", "user_id": "u1", "need_days": 3 }),
    );
    assert_eq!(r["success"], json!(false));
    assert!(text(&r, "message").contains("未开启"));
}

#[test]
fn test_approve_grants_by_requested_days() {
    let s = Server::start("approve");
    register(s.port, "app-a", "secret-a", "KEY-A", true);

    let r = s.post(
        "/api/request",
        json!({ "app_id": "app-a", "user_id": "u1", "need_days": 5, "message": "做作业" }),
    );
    assert_eq!(r["success"], json!(true));

    // 重复提交不产生第二条待审批
    let r = s.post(
        "/api/request",
        json!({ "app_id": "app-a", "user_id": "u1", "need_days": 5 }),
    );
    assert_eq!(r["success"], json!(true));

    let r = s.post(
        "/api/requests",
        json!({ "app_id": "app-a", "secret": "secret-a" }),
    );
    assert_eq!(
        r["requests"].as_array().unwrap().len(),
        1,
        "只应有一条待审批"
    );

    let r = s.post(
        "/api/approve",
        json!({ "app_id": "app-a", "secret": "secret-a", "user_id": "u1" }),
    );
    assert_eq!(r["success"], json!(true));

    let r = s.post("/api/key", json!({ "app_id": "app-a", "user_id": "u1" }));
    assert_eq!(text(&r, "key"), "KEY-A", "审批通过后应可取密钥");

    // 审批后列表里不再有该申请
    let r = s.post(
        "/api/requests",
        json!({ "app_id": "app-a", "secret": "secret-a" }),
    );
    assert_eq!(r["requests"].as_array().unwrap().len(), 0);
}

#[test]
fn test_deny_does_not_grant() {
    let s = Server::start("deny");
    register(s.port, "app-a", "secret-a", "KEY-A", true);
    s.post(
        "/api/request",
        json!({ "app_id": "app-a", "user_id": "u1", "need_days": 5 }),
    );

    let r = s.post(
        "/api/deny",
        json!({ "app_id": "app-a", "secret": "secret-a", "user_id": "u1" }),
    );
    assert_eq!(r["success"], json!(true));

    let r = s.post("/api/key", json!({ "app_id": "app-a", "user_id": "u1" }));
    assert_eq!(r["success"], json!(false), "被拒绝不应获得授权");
}

#[test]
fn test_request_short_circuits_when_already_granted() {
    let s = Server::start("already-granted");
    register(s.port, "app-a", "secret-a", "KEY-A", true);
    s.post(
        "/api/grant",
        json!({ "app_id": "app-a", "secret": "secret-a", "user_id": "u1" }),
    );
    let r = s.post(
        "/api/request",
        json!({ "app_id": "app-a", "user_id": "u1", "need_days": 5 }),
    );
    assert_eq!(r["success"], json!(true));
    assert!(text(&r, "message").contains("已有权限"));
}

// ===== 多应用隔离 =====

#[test]
fn test_requests_and_logs_are_app_scoped() {
    let s = Server::start("isolation");
    register(s.port, "app-a", "secret-a", "KEY-A", true);
    register(s.port, "app-b", "secret-b", "KEY-B", true);
    s.post(
        "/api/request",
        json!({ "app_id": "app-a", "user_id": "alice", "need_days": 1 }),
    );
    s.post(
        "/api/request",
        json!({ "app_id": "app-b", "user_id": "bob", "need_days": 1 }),
    );

    // 拿 A 的密钥查 A：只能看到 alice
    let r = s.post(
        "/api/requests",
        json!({ "app_id": "app-a", "secret": "secret-a" }),
    );
    let users: Vec<String> = r["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| text(x, "user_id"))
        .collect();
    assert_eq!(users, vec!["alice".to_string()], "查到了别的文件的申请");

    // 拿 B 的密钥冒充查 A：必须被拒
    let r = s.post(
        "/api/requests",
        json!({ "app_id": "app-a", "secret": "secret-b" }),
    );
    assert_eq!(r["success"], json!(false), "跨应用查询必须被拒绝");

    // 审计日志同理
    let r = s.post(
        "/api/logs",
        json!({ "app_id": "app-a", "secret": "secret-b" }),
    );
    assert_eq!(r["success"], json!(false));
    let r = s.post(
        "/api/logs",
        json!({ "app_id": "app-a", "secret": "secret-a" }),
    );
    assert_eq!(r["success"], json!(true));
    let actions: Vec<String> = r["logs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| text(x, "action"))
        .collect();
    assert!(actions.iter().all(|a| a != "bob"), "日志串了别的文件");
}

#[test]
fn test_audit_logs_record_success_and_failure() {
    let s = Server::start("audit");
    register(s.port, "app-a", "secret-a", "KEY-A", true);
    // 一次失败：未授权取密钥
    s.post("/api/key", json!({ "app_id": "app-a", "user_id": "u1" }));
    // 一次成功：授权后取密钥
    s.post(
        "/api/grant",
        json!({ "app_id": "app-a", "secret": "secret-a", "user_id": "u1" }),
    );
    s.post("/api/key", json!({ "app_id": "app-a", "user_id": "u1" }));

    let r = s.post(
        "/api/logs",
        json!({ "app_id": "app-a", "secret": "secret-a" }),
    );
    let logs = r["logs"].as_array().unwrap();
    assert!(logs.len() >= 4, "日志条数不足: {}", r);
    let key_logs: Vec<&Value> = logs.iter().filter(|l| text(l, "action") == "key").collect();
    assert!(
        key_logs.iter().any(|l| l["success"] == json!(false)),
        "应记录失败的取钥"
    );
    assert!(
        key_logs.iter().any(|l| l["success"] == json!(true)),
        "应记录成功的取钥"
    );
}

// ===== 启动、迁移与维护 =====

/// 造一个 v1 的库文件（与已发布版本建出的表结构一致：apps 没有 ip_whitelist 列）。
///
/// 线上部署的库正是这个形态且带着数据，升级路径必须无损，所以这里手工复刻 v1 的建表语句，
/// 不复用服务端的 MIGRATIONS——那样会把「测试跟着实现改」的假象当成验证。
fn create_v1_db(db: &Path) {
    let pool = pool_for(db);
    runtime().block_on(async {
        for stmt in [
            r#"CREATE TABLE apps (
                app_id TEXT PRIMARY KEY,
                secret TEXT NOT NULL,
                content_key TEXT NOT NULL,
                allow_temp BOOLEAN DEFAULT 0,
                created_at TEXT NOT NULL
            )"#,
            r#"CREATE TABLE grants (
                app_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                granted_at TEXT NOT NULL,
                expires_at TEXT,
                PRIMARY KEY (app_id, user_id)
            )"#,
            r#"CREATE TABLE requests (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                app_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                need_days INTEGER,
                message TEXT,
                status TEXT DEFAULT 'pending',
                created_at TEXT NOT NULL,
                resolved_at TEXT
            )"#,
            r#"CREATE TABLE audit_logs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                app_id TEXT NOT NULL,
                user_id TEXT,
                action TEXT NOT NULL,
                success INTEGER NOT NULL,
                details TEXT,
                created_at TEXT NOT NULL
            )"#,
            "CREATE INDEX idx_requests_app_status ON requests(app_id, status)",
            "CREATE INDEX idx_audit_app ON audit_logs(app_id, id)",
        ] {
            sqlx::query(stmt).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO apps (app_id, secret, content_key, allow_temp, created_at) VALUES ('app-v1', 'secret-v1', 'KEY-V1', 1, '20200101000000')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO grants (app_id, user_id, granted_at, expires_at) VALUES ('app-v1', 'u1', '20200101', NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO audit_logs (app_id, user_id, action, success, details, created_at) VALUES ('app-v1', 'u1', 'grant', 1, '授权成功', '20200101000000')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("PRAGMA user_version = 1")
            .execute(&pool)
            .await
            .unwrap();
    });
    runtime().block_on(pool.close());
}

#[test]
fn test_v1_database_upgrades_without_data_loss() {
    let dir = temp_dir("upgrade-v1");
    let db = dir.join("secunzip.db");
    create_v1_db(&db);

    let s = Server::start_in_dir(&dir, ROOT_URL, &[]);

    // 老数据仍在：管理员取钥与已授权用户取钥都成功
    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-v1", "secret": "secret-v1" }),
    );
    assert_eq!(text(&r, "key"), "KEY-V1", "v1 升级后旧数据丢失: {}", r);
    let r = s.post("/api/key", json!({ "app_id": "app-v1", "user_id": "u1" }));
    assert_eq!(text(&r, "key"), "KEY-V1", "v1 升级后授权丢失: {}", r);

    // 版本升到 v2，旧行的新列为 NULL（= 不限制）
    let pool = pool_for(&db);
    let (ver, wl, grant_logs): (i64, Option<String>, i64) = runtime().block_on(async {
        let ver = sqlx::query("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
        let wl = sqlx::query("SELECT ip_whitelist FROM apps WHERE app_id = 'app-v1'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get::<Option<String>, _>("ip_whitelist");
        let grant_logs = sqlx::query(
            "SELECT COUNT(*) AS n FROM audit_logs WHERE app_id = 'app-v1' AND action = 'grant'",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("n");
        (ver, wl, grant_logs)
    });
    runtime().block_on(pool.close());
    assert_eq!(ver, 2, "v1 库应升级到 v2");
    assert_eq!(wl, None, "v1 旧行的 ip_whitelist 应为空（不限制）");
    assert_eq!(grant_logs, 1, "v1 库的审计日志被清掉了");

    // 升级后的库照常支持新字段
    let r = register_with_ip_whitelist(
        s.port,
        "app-new",
        "secret-new",
        "KEY-NEW",
        false,
        json!(["127.0.0.0/8"]),
    );
    assert_eq!(r["success"], json!(true), "升级后注册失败: {}", r);
    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-new", "secret": "secret-new" }),
    );
    assert_eq!(text(&r, "key"), "KEY-NEW");
}

#[test]
fn test_restart_keeps_data_and_migration_idempotent() {
    let s = Server::start("restart");
    register_with_ip_whitelist(
        s.port,
        "app-a",
        "secret-a",
        "KEY-A",
        true,
        json!(["127.0.0.0/8"]),
    );
    // 一个没有白名单的应用，验证 v2 新列可为 NULL 且不影响取钥
    register(s.port, "app-b", "secret-b", "KEY-B", false);
    let db = s.db();
    let port = s.port;
    let mut s = s;
    s.stop();

    // 同一个库再起一次，不能报错也不能丢数据
    let dir = s.dir.clone();
    let mut cmd = Command::new(BIN);
    let child = cmd
        .current_dir(&dir)
        .env("SECUNZIP_PORT", port.to_string())
        .env("DATABASE_URL", ROOT_URL)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    s.dir = dir;
    s.child = Some(child);
    s.wait_ready();

    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-a", "secret": "secret-a" }),
    );
    assert_eq!(text(&r, "key"), "KEY-A", "重启后数据丢失");
    let r = s.post(
        "/api/key",
        json!({ "app_id": "app-b", "secret": "secret-b" }),
    );
    assert_eq!(text(&r, "key"), "KEY-B", "重启后数据丢失");

    let pool = pool_for(&db);
    let (ver, wl): (i64, Option<String>) = runtime().block_on(async {
        let ver = sqlx::query("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
        let wl = sqlx::query("SELECT ip_whitelist FROM apps WHERE app_id = 'app-a'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get::<Option<String>, _>("ip_whitelist");
        (ver, wl)
    });
    runtime().block_on(pool.close());
    assert_eq!(ver, 2, "schema 版本应为 2");
    assert_eq!(
        wl.as_deref(),
        Some(r#"["127.0.0.0/8"]"#),
        "重启后 ip_whitelist 丢失"
    );
}

#[test]
fn test_database_parent_dirs_are_created() {
    let dir = temp_dir("nested-db");
    let port = free_port();
    let mut child = Command::new(BIN)
        .current_dir(&dir)
        .env("SECUNZIP_PORT", port.to_string())
        .env("DATABASE_URL", "sqlite:data/sub/secunzip.db?mode=rwc")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let db = dir.join("data").join("sub").join("secunzip.db");
    while Instant::now() < deadline && !db.exists() {
        std::thread::sleep(Duration::from_millis(50));
    }
    let ok = db.exists();
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(ok, "DATABASE_URL 里的多级父目录没有被创建");
}

#[test]
fn test_audit_prune_keeps_newest() {
    let s = Server::start("prune");
    register(s.port, "app-a", "secret-a", "KEY-A", true);
    let db = s.db();
    let port = s.port;
    let mut s = s;
    s.stop();

    let pool = pool_for(&db);
    runtime().block_on(async {
        for i in 0..30 {
            sqlx::query(
                "INSERT INTO audit_logs (app_id, user_id, action, success, details, created_at) VALUES ('app-a', 'u', 'pad', 1, ?, '20200101')",
            )
            .bind(format!("第 {} 条", i))
            .execute(&pool)
            .await
            .unwrap();
        }
        // 另一个应用不应被牵连
        for i in 0..30 {
            sqlx::query(
                "INSERT INTO audit_logs (app_id, user_id, action, success, details, created_at) VALUES ('app-b', 'u', 'pad', 1, ?, '20200101')",
            )
            .bind(format!("b-{}", i))
            .execute(&pool)
            .await
            .unwrap();
        }
    });
    runtime().block_on(pool.close());

    // 以保留 10 条的配置重启，启动时清理一次
    let dir = s.dir.clone();
    let mut cmd = Command::new(BIN);
    let child = cmd
        .current_dir(&dir)
        .env("SECUNZIP_PORT", port.to_string())
        .env("DATABASE_URL", ROOT_URL)
        .env("SECUNZIP_AUDIT_KEEP", "10")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    s.dir = dir;
    s.child = Some(child);
    s.wait_ready();

    let pool = pool_for(&db);
    let (a, b): (i64, i64) = runtime().block_on(async {
        let a = sqlx::query("SELECT COUNT(*) AS n FROM audit_logs WHERE app_id = 'app-a'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("n");
        let b = sqlx::query("SELECT COUNT(*) AS n FROM audit_logs WHERE app_id = 'app-b'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("n");
        (a, b)
    });
    // 留下的必须是最后插入的那些
    let last: String = runtime().block_on(async {
        sqlx::query(
            "SELECT details FROM audit_logs WHERE app_id = 'app-a' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("details")
    });
    runtime().block_on(pool.close());

    assert_eq!(a, 10, "app-a 应只保留 10 条");
    assert_eq!(b, 10, "app-b 应只保留 10 条");
    assert_eq!(last, "第 29 条", "保留的应该是最近的记录");
}

#[test]
fn test_backup_writes_usable_snapshot() {
    let s = Server::start("backup");
    register(s.port, "app-a", "secret-a", "KEY-A", true);
    let dir = s.dir.clone();
    let mut s = s;
    s.stop();

    let out = dir.join("snapshot.db");
    let (ok, stderr) = run_bin(
        &dir,
        &["--backup", out.to_str().unwrap()],
        &[("DATABASE_URL", ROOT_URL)],
    );
    assert!(ok, "备份失败: {}", stderr);
    assert!(
        out.exists() && out.metadata().unwrap().len() > 0,
        "备份文件为空"
    );

    // 快照里应该有注册的数据
    let pool = pool_for(&out);
    let n: i64 = runtime().block_on(async {
        sqlx::query("SELECT COUNT(*) AS n FROM apps")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("n")
    });
    runtime().block_on(pool.close());
    assert_eq!(n, 1, "快照里缺少数据");
}

#[test]
fn test_refuses_newer_schema() {
    let dir = temp_dir("too-new");
    let db = dir.join("secunzip.db");
    // 先建库并把 schema 版本抬到 99
    let pool = pool_for(&db);
    runtime().block_on(async {
        sqlx::query("PRAGMA user_version = 99")
            .execute(&pool)
            .await
            .unwrap();
    });
    runtime().block_on(pool.close());

    let (ok, stderr) = run_bin(
        &dir,
        &[],
        &[
            ("DATABASE_URL", ROOT_URL),
            ("SECUNZIP_PORT", &free_port().to_string()),
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);

    assert!(!ok, "高于程序支持的库不应启动成功");
    assert!(
        stderr.contains("高于本程序支持"),
        "错误信息不明确: {}",
        stderr
    );
}

#[test]
fn test_unusable_port_exits_with_error() {
    // 不用"端口被占用"来构造失败：Windows 上 127.0.0.1:P 被占时，服务端绑 0.0.0.0:P 依然成功，
    // 进程不会退出，测试会永久挂起。改成给一个无法解析的端口，失败是确定的。
    let dir = temp_dir("bad-port");

    let (ok, stderr) = run_bin(
        &dir,
        &[],
        &[("DATABASE_URL", ROOT_URL), ("SECUNZIP_PORT", "not-a-port")],
    );
    let _ = std::fs::remove_dir_all(&dir);

    assert!(!ok, "端口不可用时应退出");
    assert!(stderr.contains("无法绑定"), "错误信息不明确: {}", stderr);
}

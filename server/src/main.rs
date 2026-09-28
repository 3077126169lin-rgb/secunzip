use axum::{
    extract::{ConnectInfo, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions, SqliteRow};
use sqlx::Row;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

type Db = SqlitePool;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();

    // 数据库位置：优先读 DATABASE_URL（部署脚本与 Docker 卷靠它指到数据盘），
    // 未设置时回落到工作目录下的 secunzip.db。仅处理 sqlite: 形式。
    let db_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:secunzip.db?mode=rwc".into());
    if let Some(rest) = db_url.strip_prefix("sqlite:") {
        let path = rest.split('?').next().unwrap_or("");
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
    }

    let db = SqlitePoolOptions::new()
        .connect(&db_url)
        .await
        .expect("数据库连接失败");

    // 备份模式：secunzip-server --backup <输出文件>
    // 用 VACUUM INTO 生成一致性快照，可在服务端运行期间执行。
    if let Some(i) = args.iter().position(|a| a == "--backup") {
        let Some(out) = args.get(i + 1) else {
            eprintln!("用法: secunzip-server --backup <输出文件>");
            std::process::exit(2);
        };
        match sqlx::query("VACUUM INTO ?").bind(out).execute(&db).await {
            Ok(_) => println!("已备份到 {}", out),
            Err(e) => {
                eprintln!("备份失败: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    // 建表 / 升级 schema，并做一次审计日志保留清理
    if let Err(e) = migrate(&db).await {
        eprintln!("数据库初始化失败: {}", e);
        std::process::exit(1);
    }
    prune_audit(&db).await;
    tokio::spawn({
        let db = db.clone();
        async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
            tick.tick().await; // 跳过立即触发的那一次
            loop {
                tick.tick().await;
                prune_audit(&db).await;
            }
        }
    });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/api/register", post(register_app))
        .route("/api/grant", post(grant_user))
        .route("/api/revoke", post(revoke_user))
        .route("/api/key", post(get_key))
        .route("/api/request", post(request_access))
        .route("/api/requests", post(list_requests))
        .route("/api/logs", post(list_logs))
        .route("/api/approve", post(approve_request))
        .route("/api/deny", post(deny_request))
        .with_state(db);

    let port = std::env::var("SECUNZIP_PORT").unwrap_or_else(|_| "8090".into());
    let addr = format!("0.0.0.0:{}", port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "无法绑定 {}: {}（端口可能被占用，可设 SECUNZIP_PORT 环境变量换端口）",
                addr, e
            );
            std::process::exit(1);
        }
    };
    println!("SecUnzip Server on http://0.0.0.0:{}", port);
    // 必须带 ConnectInfo 才能拿到对端真实 IP：/api/key 的 IP 白名单靠它判定来源。
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

// ===== 数据库 schema 与维护 =====

/// 当前 schema 版本，存于 `PRAGMA user_version`（该值随库文件持久化，不随连接）。
const SCHEMA_VERSION: i64 = 2;

/// 顺序迁移表：每项 (版本, 语句列表)。
/// v1 的建表/建索引语句都带 IF NOT EXISTS，对已有库幂等；v2 的加列没有该语法，见下。
const MIGRATIONS: &[(i64, &[&str])] = &[
    (
        1,
        &[
            r#"CREATE TABLE IF NOT EXISTS apps (
            app_id TEXT PRIMARY KEY,
            secret TEXT NOT NULL,
            content_key TEXT NOT NULL,
            allow_temp BOOLEAN DEFAULT 0,
            created_at TEXT NOT NULL
        )"#,
            r#"CREATE TABLE IF NOT EXISTS grants (
            app_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            granted_at TEXT NOT NULL,
            expires_at TEXT,
            PRIMARY KEY (app_id, user_id)
        )"#,
            r#"CREATE TABLE IF NOT EXISTS requests (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            app_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            need_days INTEGER,
            message TEXT,
            status TEXT DEFAULT 'pending',
            created_at TEXT NOT NULL,
            resolved_at TEXT
        )"#,
            r#"CREATE TABLE IF NOT EXISTS audit_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            app_id TEXT NOT NULL,
            user_id TEXT,
            action TEXT NOT NULL,
            success INTEGER NOT NULL,
            details TEXT,
            created_at TEXT NOT NULL
        )"#,
            "CREATE INDEX IF NOT EXISTS idx_requests_app_status ON requests(app_id, status)",
            "CREATE INDEX IF NOT EXISTS idx_audit_app ON audit_logs(app_id, id)",
        ],
    ),
    // v2：apps 增加 ip_whitelist（JSON 数组文本，NULL / 空数组 = 不限制来源 IP）。
    // SQLite 没有 ADD COLUMN IF NOT EXISTS；user_version 顺序门禁已保证该语句不会重跑，
    // migrate() 另外会先查 PRAGMA table_info，列已存在时跳过，避免上次迁移在
    // 「加列成功、写版本号失败」之间中断后无法再次启动。
    (2, &["ALTER TABLE apps ADD COLUMN ip_whitelist TEXT"]),
];

/// 按 `PRAGMA user_version` 逐级应用迁移；版本高于本程序时拒绝启动。
async fn migrate(db: &Db) -> Result<(), sqlx::Error> {
    let row = sqlx::query("PRAGMA user_version").fetch_one(db).await?;
    let mut current: i64 = row.get(0);
    if current > SCHEMA_VERSION {
        eprintln!(
            "数据库 schema 为 v{}，高于本程序支持的 v{}，请升级服务端",
            current, SCHEMA_VERSION
        );
        std::process::exit(1);
    }
    for (ver, stmts) in MIGRATIONS {
        if *ver <= current {
            continue;
        }
        for s in *stmts {
            // 加列语句没有 IF NOT EXISTS，单独做一次存在性判断（见 MIGRATIONS 中的说明）
            if let Some(rest) = s.strip_prefix("ALTER TABLE apps ADD COLUMN ") {
                let col = rest.split_whitespace().next().unwrap_or("");
                if column_exists(db, "apps", col).await? {
                    continue;
                }
            }
            sqlx::query(s).execute(db).await?;
        }
        sqlx::query(&format!("PRAGMA user_version = {}", ver))
            .execute(db)
            .await?;
        println!("数据库 schema 已迁移到 v{}", ver);
        current = *ver;
    }
    Ok(())
}

/// 表中是否已有该列（用于给没有 IF NOT EXISTS 的 ALTER TABLE 兜底）
async fn column_exists(db: &Db, table: &str, column: &str) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(&format!("PRAGMA table_info({})", table))
        .fetch_all(db)
        .await?;
    Ok(rows.iter().any(|r| r.get::<String, _>("name") == column))
}

/// 每个 app_id 保留的审计日志条数，可用 `SECUNZIP_AUDIT_KEEP` 覆盖
fn audit_keep() -> i64 {
    std::env::var("SECUNZIP_AUDIT_KEEP")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1000)
}

/// 审计日志保留策略：每个 app_id 只保留最近 N 条，避免无限增长
async fn prune_audit(db: &Db) {
    let _ = sqlx::query(
        "DELETE FROM audit_logs WHERE id IN (
            SELECT id FROM (
                SELECT id, ROW_NUMBER() OVER (PARTITION BY app_id ORDER BY id DESC) AS rn
                FROM audit_logs
            ) WHERE rn > ?
        )",
    )
    .bind(audit_keep())
    .execute(db)
    .await;
}

// ===== 请求/响应 =====

#[derive(Deserialize)]
struct RegisterRequest {
    app_id: String,
    secret: String,
    content_key: String,
    allow_temp: Option<bool>,
    /// 允许取钥的来源 IP 列表（IPv4 单机或 CIDR）。缺省 / 空数组 = 不限制。
    #[serde(default)]
    ip_whitelist: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct GrantRequest {
    app_id: String,
    secret: String,
    user_id: String,
    expires_at: Option<String>, // YYYYMMDD 或 Nd
}

#[derive(Deserialize)]
struct KeyRequest {
    app_id: String,
    #[serde(default)]
    user_id: Option<String>,
    /// 管理员密钥：有则免申请直接取密钥
    #[serde(default)]
    secret: Option<String>,
}

#[derive(Deserialize)]
struct AccessRequest {
    app_id: String,
    user_id: String,
    need_days: Option<i32>,
    message: Option<String>,
}

#[derive(Deserialize)]
struct ApproveRequest {
    app_id: String,
    secret: String,
    user_id: String,
    expires_at: Option<String>,
}

#[derive(Deserialize)]
struct ListRequestsRequest {
    app_id: String,
    secret: String,
}

#[derive(Deserialize)]
struct LogsRequest {
    app_id: String,
    secret: String,
}

#[derive(Serialize)]
struct LogItem {
    user_id: Option<String>,
    action: String,
    success: bool,
    details: Option<String>,
    created_at: String,
}

#[derive(Serialize)]
struct ApiResponse {
    success: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    requests: Option<Vec<RequestItem>>,
}

#[derive(Serialize)]
struct RequestItem {
    user_id: String,
    need_days: Option<i32>,
    message: Option<String>,
    created_at: String,
}

fn ok(msg: &str) -> Json<ApiResponse> {
    Json(ApiResponse {
        success: true,
        message: msg.into(),
        key: None,
        requests: None,
    })
}
fn err(msg: &str) -> Json<ApiResponse> {
    Json(ApiResponse {
        success: false,
        message: msg.into(),
        key: None,
        requests: None,
    })
}

// ===== 处理函数 =====

/// 健康检查：容器编排 / 端口转发器用的存活探针。
///
/// 不访问数据库、不校验任何凭据，也不返回除固定状态外的信息，
/// 因此不会因为库故障而失败，也不会泄露部署细节。
async fn healthz() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

/// 注册应用（打包者调用）：存 app_id / secret / content_key / allow_temp / ip_whitelist
///
/// 防覆盖：app_id = 打包文件 MD5，任何拿到文件的人都能公开推导出来。
/// 若无校验，任何人都能 register 覆盖他人文件的 content_key / secret（使文件变砖或接管管理权）。
/// 因此：app_id 已存在时，只有持相同 secret 的同一打包者才允许更新，否则拒绝。
async fn register_app(State(db): State<Db>, Json(req): Json<RegisterRequest>) -> Json<ApiResponse> {
    // 白名单在入库前校验：非法条目直接拒绝，避免把无法匹配的条目写进库。
    // 防御性说明：即使库里被人工改出非法条目，取钥时的匹配也会忽略它并按失败关闭处理。
    let ip_whitelist = match &req.ip_whitelist {
        None => None,
        Some(list) => {
            if let Some(bad) = list.iter().find(|e| !valid_ip_entry(e)) {
                log_audit(
                    &db,
                    &req.app_id,
                    "",
                    "register",
                    false,
                    &format!("IP 白名单条目非法: {}", bad),
                )
                .await;
                return err(&format!(
                    "IP 白名单条目非法（仅支持 IPv4 单机或 CIDR，如 203.0.113.7、203.0.113.0/24）: {}",
                    bad
                ));
            }
            // 空数组与缺省等价，统一存 NULL
            if list.is_empty() {
                None
            } else {
                Some(serde_json::to_string(list).unwrap())
            }
        }
    };

    let existing: Option<SqliteRow> = sqlx::query("SELECT secret FROM apps WHERE app_id = ?")
        .bind(&req.app_id)
        .fetch_optional(&db)
        .await
        .unwrap();
    if let Some(row) = existing {
        let stored: String = row.get("secret");
        if stored != req.secret {
            log_audit(
                &db,
                &req.app_id,
                "",
                "register",
                false,
                "app_id 已被占用，拒绝覆盖",
            )
            .await;
            return err("该文件 ID 已被登记且密钥不匹配，拒绝覆盖");
        }
    }
    let now = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let allow_temp = if req.allow_temp.unwrap_or(false) {
        1i64
    } else {
        0i64
    };
    sqlx::query("INSERT OR REPLACE INTO apps (app_id, secret, content_key, allow_temp, ip_whitelist, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&req.app_id)
        .bind(&req.secret)
        .bind(&req.content_key)
        .bind(allow_temp)
        .bind(&ip_whitelist)
        .bind(&now)
        .execute(&db).await.unwrap();
    log_audit(&db, &req.app_id, "", "register", true, "注册成功").await;
    ok("应用已注册")
}

/// 验证管理员密钥
async fn verify_secret(db: &Db, app_id: &str, secret: &str) -> bool {
    let row: Option<SqliteRow> =
        sqlx::query("SELECT 1 as x FROM apps WHERE app_id = ? AND secret = ?")
            .bind(app_id)
            .bind(secret)
            .fetch_optional(db)
            .await
            .unwrap();
    row.is_some()
}

/// 写审计日志（失败不阻断主流程）
async fn log_audit(
    db: &Db,
    app_id: &str,
    user_id: &str,
    action: &str,
    success: bool,
    details: &str,
) {
    let now = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let _ = sqlx::query("INSERT INTO audit_logs (app_id, user_id, action, success, details, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(app_id)
        .bind(user_id)
        .bind(action)
        .bind(if success { 1i64 } else { 0i64 })
        .bind(details)
        .bind(&now)
        .execute(db).await;
}

/// 授权用户（管理员）
async fn grant_user(State(db): State<Db>, Json(req): Json<GrantRequest>) -> Json<ApiResponse> {
    if !verify_secret(&db, &req.app_id, &req.secret).await {
        return err("密钥错误");
    }
    let now = chrono::Utc::now().format("%Y%m%d").to_string();
    let expires = req.expires_at.as_deref().and_then(parse_expire);
    sqlx::query("INSERT OR REPLACE INTO grants (app_id, user_id, granted_at, expires_at) VALUES (?, ?, ?, ?)")
        .bind(&req.app_id)
        .bind(&req.user_id)
        .bind(&now)
        .bind(expires)
        .execute(&db).await.unwrap();
    log_audit(&db, &req.app_id, &req.user_id, "grant", true, "授权成功").await;
    ok(&format!("已授权 {}", req.user_id))
}

/// 吊销用户（管理员）
async fn revoke_user(State(db): State<Db>, Json(req): Json<GrantRequest>) -> Json<ApiResponse> {
    if !verify_secret(&db, &req.app_id, &req.secret).await {
        return err("无权操作");
    }
    sqlx::query("DELETE FROM grants WHERE app_id = ? AND user_id = ?")
        .bind(&req.app_id)
        .bind(&req.user_id)
        .execute(&db)
        .await
        .unwrap();
    log_audit(&db, &req.app_id, &req.user_id, "revoke", true, "吊销成功").await;
    ok(&format!("已吊销 {}", req.user_id))
}

/// 获取解密密钥：管理员凭 secret 直接取；否则验证授权未过期后下发 content_key
///
/// IP 白名单对管理员路径同样生效：白名单表达的是「这份密钥允许从哪里取」，
/// 而管理员持有 secret 本来就能重新 register 并改写/清空白名单，
/// 所以给管理员开绕行口子不增加任何防锁定能力，只会削弱限制本身，故两条路径统一校验。
async fn get_key(
    State(db): State<Db>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(req): Json<KeyRequest>,
) -> Json<ApiResponse> {
    // 来源 IP 必须是 TCP 连接的对端地址（ConnectInfo），不接受请求体里自报的 IP：
    // 客户端可以随意伪造请求体字段，但对端地址由内核填充。
    let peer_ip = peer.ip();
    let app_id = req.app_id.clone();
    if let Err(reason) = check_source_ip(&db, &app_id, peer_ip).await {
        log_audit(
            &db,
            &app_id,
            if req.secret.is_some() {
                "admin"
            } else {
                req.user_id.as_deref().unwrap_or("")
            },
            "key",
            false,
            &reason,
        )
        .await;
        return err(&format!("{}，拒绝下发密钥", reason));
    }

    // 管理员：凭 secret 直接取密钥，无需申请/授权
    if let Some(secret) = &req.secret {
        if verify_secret(&db, &req.app_id, secret).await {
            let app: Option<SqliteRow> =
                sqlx::query("SELECT content_key FROM apps WHERE app_id = ?")
                    .bind(&req.app_id)
                    .fetch_optional(&db)
                    .await
                    .unwrap();
            return match app {
                Some(a) => {
                    let key: String = a.get("content_key");
                    log_audit(&db, &req.app_id, "admin", "key", true, "管理员取钥").await;
                    Json(ApiResponse {
                        success: true,
                        message: "管理员".into(),
                        key: Some(key),
                        requests: None,
                    })
                }
                None => err("应用不存在"),
            };
        }
    }

    // 普通用户：查授权（expires_at 为空 = 永久）
    let Some(user_id) = &req.user_id else {
        return err("未授权，请申请临时权限或联系管理员");
    };
    let now = chrono::Utc::now().format("%Y%m%d").to_string();

    // 查授权
    let grant: Option<SqliteRow> =
        sqlx::query("SELECT expires_at FROM grants WHERE app_id = ? AND user_id = ?")
            .bind(&req.app_id)
            .bind(user_id)
            .fetch_optional(&db)
            .await
            .unwrap();

    match grant {
        Some(g) => {
            let expires: Option<String> = g.get("expires_at");
            if let Some(exp) = &expires {
                if now > *exp {
                    sqlx::query("DELETE FROM grants WHERE app_id = ? AND user_id = ?")
                        .bind(&req.app_id)
                        .bind(user_id)
                        .execute(&db)
                        .await
                        .unwrap();
                    log_audit(&db, &req.app_id, user_id, "key", false, "授权已过期").await;
                    return err(&format!("权限已过期（{}）", exp));
                }
            }
            // expires 为空 → 永久有效
        }
        None => {
            log_audit(&db, &req.app_id, user_id, "key", false, "未授权").await;
            return err("未授权，请申请临时权限或联系管理员");
        }
    }

    // 下发 content_key
    let app: Option<SqliteRow> = sqlx::query("SELECT content_key FROM apps WHERE app_id = ?")
        .bind(&req.app_id)
        .fetch_optional(&db)
        .await
        .unwrap();

    match app {
        Some(a) => {
            let key: String = a.get("content_key");
            log_audit(&db, &req.app_id, user_id, "key", true, "下发密钥").await;
            Json(ApiResponse {
                success: true,
                message: "成功".into(),
                key: Some(key),
                requests: None,
            })
        }
        None => err("应用不存在"),
    }
}

/// 申请临时权限（用户）
async fn request_access(State(db): State<Db>, Json(req): Json<AccessRequest>) -> Json<ApiResponse> {
    // 已授权则无需申请
    let existing: Option<SqliteRow> =
        sqlx::query("SELECT 1 as x FROM grants WHERE app_id = ? AND user_id = ?")
            .bind(&req.app_id)
            .bind(&req.user_id)
            .fetch_optional(&db)
            .await
            .unwrap();
    if existing.is_some() {
        return ok("已有权限，直接打开即可");
    }

    // 检查是否开启临时申请
    let app: Option<SqliteRow> = sqlx::query("SELECT allow_temp FROM apps WHERE app_id = ?")
        .bind(&req.app_id)
        .fetch_optional(&db)
        .await
        .unwrap();
    match app {
        Some(a) => {
            let at: Option<i64> = a.get("allow_temp");
            if at.unwrap_or(0) == 0 {
                return err("该文件未开启临时权限申请");
            }
        }
        None => return err("应用不存在"),
    }

    // 避免重复待审批
    let pending: Option<SqliteRow> = sqlx::query(
        "SELECT 1 as x FROM requests WHERE app_id = ? AND user_id = ? AND status = 'pending'",
    )
    .bind(&req.app_id)
    .bind(&req.user_id)
    .fetch_optional(&db)
    .await
    .unwrap();
    if pending.is_some() {
        return ok("已有待审批申请，请耐心等待");
    }

    let now = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    sqlx::query("INSERT INTO requests (app_id, user_id, need_days, message, status, created_at) VALUES (?, ?, ?, ?, 'pending', ?)")
        .bind(&req.app_id).bind(&req.user_id)
        .bind(req.need_days).bind(&req.message).bind(&now)
        .execute(&db).await.unwrap();
    log_audit(
        &db,
        &req.app_id,
        &req.user_id,
        "request",
        true,
        "提交临时权限申请",
    )
    .await;
    ok("已提交申请，请等待管理员审批")
}

/// 待审批列表（管理员查看：需该文件的 secret，且只返回本文件的申请）
async fn list_requests(
    State(db): State<Db>,
    Json(req): Json<ListRequestsRequest>,
) -> Json<ApiResponse> {
    if !verify_secret(&db, &req.app_id, &req.secret).await {
        return err("无权查看（密钥错误）");
    }
    let rows = sqlx::query("SELECT user_id, need_days, message, created_at FROM requests WHERE app_id = ? AND status = 'pending' ORDER BY created_at DESC")
        .bind(&req.app_id)
        .fetch_all(&db).await.unwrap();

    let requests: Vec<RequestItem> = rows
        .iter()
        .map(|r| RequestItem {
            user_id: r.get("user_id"),
            need_days: r.get("need_days"),
            message: r.get("message"),
            created_at: r.get("created_at"),
        })
        .collect();

    Json(ApiResponse {
        success: true,
        message: "成功".into(),
        key: None,
        requests: Some(requests),
    })
}

/// 审计日志（管理员查看：需该文件的 secret）
async fn list_logs(State(db): State<Db>, Json(req): Json<LogsRequest>) -> Json<serde_json::Value> {
    if !verify_secret(&db, &req.app_id, &req.secret).await {
        return Json(serde_json::json!({ "success": false, "message": "无权查看（密钥错误）" }));
    }
    let rows = sqlx::query("SELECT user_id, action, success, details, created_at FROM audit_logs WHERE app_id = ? ORDER BY id DESC LIMIT 200")
        .bind(&req.app_id)
        .fetch_all(&db).await.unwrap();
    let logs: Vec<LogItem> = rows
        .iter()
        .map(|r| LogItem {
            user_id: r.get("user_id"),
            action: r.get("action"),
            success: r.get::<i64, _>("success") != 0,
            details: r.get("details"),
            created_at: r.get("created_at"),
        })
        .collect();
    Json(serde_json::json!({ "success": true, "message": "成功", "logs": logs }))
}

/// 审批通过（管理员）
async fn approve_request(
    State(db): State<Db>,
    Json(req): Json<ApproveRequest>,
) -> Json<ApiResponse> {
    if !verify_secret(&db, &req.app_id, &req.secret).await {
        return err("无权操作");
    }

    let request: Option<SqliteRow> = sqlx::query(
        "SELECT need_days FROM requests WHERE app_id = ? AND user_id = ? AND status = 'pending'",
    )
    .bind(&req.app_id)
    .bind(&req.user_id)
    .fetch_optional(&db)
    .await
    .unwrap();

    let need_days: Option<i32> = request.and_then(|r| r.get("need_days"));

    // 计算过期时间：优先用管理员指定，否则按申请天数，再否则永久
    let expires = req
        .expires_at
        .as_deref()
        .and_then(parse_expire)
        .or_else(|| need_days.map(|d| days_from_now(d as i64)));

    let now = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    sqlx::query("UPDATE requests SET status = 'approved', resolved_at = ? WHERE app_id = ? AND user_id = ? AND status = 'pending'")
        .bind(&now).bind(&req.app_id).bind(&req.user_id)
        .execute(&db).await.unwrap();

    let granted = chrono::Utc::now().format("%Y%m%d").to_string();
    sqlx::query("INSERT OR REPLACE INTO grants (app_id, user_id, granted_at, expires_at) VALUES (?, ?, ?, ?)")
        .bind(&req.app_id).bind(&req.user_id).bind(&granted).bind(&expires)
        .execute(&db).await.unwrap();

    log_audit(&db, &req.app_id, &req.user_id, "approve", true, "审批通过").await;
    ok(&format!("已通过 {}", req.user_id))
}

/// 审批拒绝（管理员）
async fn deny_request(State(db): State<Db>, Json(req): Json<ApproveRequest>) -> Json<ApiResponse> {
    if !verify_secret(&db, &req.app_id, &req.secret).await {
        return err("无权操作");
    }
    let now = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    sqlx::query("UPDATE requests SET status = 'denied', resolved_at = ? WHERE app_id = ? AND user_id = ? AND status = 'pending'")
        .bind(&now).bind(&req.app_id).bind(&req.user_id)
        .execute(&db).await.unwrap();
    log_audit(&db, &req.app_id, &req.user_id, "deny", true, "审批拒绝").await;
    ok(&format!("已拒绝 {}", req.user_id))
}

// ===== 工具函数 =====

/// 单条白名单条目是否匹配给定 IPv4 地址。
///
/// 支持 `203.0.113.7`（单机，等价 /32）与 `203.0.113.0/24`（CIDR，前缀 0..=32）。
/// 只解析 IPv4：IPv6 条目（如 `2001:db8::1`）或任何非法写法一律返回 None，
/// 由调用方按「不匹配」处理并告警。条目两端的空白被忽略。
fn entry_matches(entry: &str, ip: Ipv4Addr) -> Option<bool> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    let (addr, prefix) = match entry.split_once('/') {
        Some((a, p)) => {
            let p: u32 = p.trim().parse().ok()?;
            if p > 32 {
                return None;
            }
            (a.trim(), p)
        }
        None => (entry, 32),
    };
    let net: Ipv4Addr = addr.parse().ok()?;
    let mask: u32 = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    Some(u32::from(net) & mask == u32::from(ip) & mask)
}

/// 注册时校验白名单条目：能被 entry_matches 解析即为合法（与取钥时的匹配逻辑同源）
fn valid_ip_entry(entry: &str) -> bool {
    entry_matches(entry, Ipv4Addr::UNSPECIFIED).is_some()
}

/// 读取某应用的来源 IP 白名单。
///
/// 返回 None 表示不限制（列为 NULL 或空串、或存的是空数组）；
/// 返回 Some(条目) 表示限制开启，其中条目可能为空（表示库里存了无法解析的内容）。
async fn load_whitelist(db: &Db, app_id: &str) -> Option<Vec<String>> {
    let row: SqliteRow = sqlx::query("SELECT ip_whitelist FROM apps WHERE app_id = ?")
        .bind(app_id)
        .fetch_optional(db)
        .await
        .unwrap()?;
    let raw: Option<String> = row.get("ip_whitelist");
    let raw = match raw {
        None => return None,
        Some(s) if s.trim().is_empty() => return None,
        Some(s) => s,
    };
    match serde_json::from_str::<Vec<String>>(&raw) {
        Ok(v) if v.is_empty() => None,
        Ok(v) => Some(v),
        // 库里存了无法解析的内容（正常流程写不进来，只可能被人工改动）：
        // 打印告警并失败关闭，返回空条目集 → 所有来源都不匹配 → 拒绝下发密钥。
        Err(e) => {
            eprintln!(
                "应用 {} 的 ip_whitelist 无法解析（{}），按拒绝处理: {}",
                app_id, e, raw
            );
            Some(Vec::new())
        }
    }
}

/// 取钥前的来源 IP 校验：Ok(()) 放行；Err(原因) 拒绝。
///
/// 语义：
/// - 未设白名单（NULL / 空数组）→ 不限制，任何来源都放行，老部署行为不变；
/// - IPv4 来源命中任一条目 → 放行；
/// - IPv4 来源未命中 → 拒绝；
/// - 条目非法（含 IPv6）→ 忽略该条目并告警；若因此没有任何可用条目，则全部拒绝（失败关闭）；
/// - IPv6 来源地址 → 本服务端不实现 IPv6 匹配，白名单非空时一律拒绝（失败关闭）。
async fn check_source_ip(db: &Db, app_id: &str, ip: IpAddr) -> Result<(), String> {
    let Some(entries) = load_whitelist(db, app_id).await else {
        return Ok(());
    };
    let allowed = match ip {
        IpAddr::V4(v4) => entries.iter().any(|e| match entry_matches(e, v4) {
            Some(matched) => matched,
            None => {
                eprintln!("忽略非法的 IP 白名单条目: {:?}（应用 {}）", e, app_id);
                false
            }
        }),
        IpAddr::V6(_) => {
            eprintln!(
                "来源 IP {} 为 IPv6，服务端白名单仅支持 IPv4，按拒绝处理（应用 {}）",
                ip, app_id
            );
            false
        }
    };
    if allowed {
        Ok(())
    } else {
        Err(format!("来源 IP {} 不在该文件的 IP 白名单内", ip))
    }
}

/// 解析过期时间：支持 `Nd`（N天后）、`YYYYMMDD`、`永久`（=无过期）
fn parse_expire(input: &str) -> Option<String> {
    let input = input.trim();
    // 空 / 永久 / permanent → 无过期（永久授权）
    if input.is_empty()
        || input == "永久"
        || input == "永久有效"
        || input.eq_ignore_ascii_case("permanent")
    {
        return None;
    }
    if let Some(days) = input.strip_suffix('d').or_else(|| input.strip_suffix('D')) {
        if let Ok(n) = days.parse::<i64>() {
            return Some(days_from_now(n));
        }
    }
    Some(input.to_string())
}

fn days_from_now(days: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::days(days))
        .format("%Y%m%d")
        .to_string()
}

#[cfg(test)]
mod ip_whitelist_tests {
    use super::{entry_matches, valid_ip_entry};
    use std::net::Ipv4Addr;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn test_entry_matches_single_host() {
        assert_eq!(entry_matches("203.0.113.7", ip("203.0.113.7")), Some(true));
        assert_eq!(entry_matches("203.0.113.7", ip("203.0.113.8")), Some(false));
        // 两端空白被忽略
        assert_eq!(
            entry_matches(" 203.0.113.7 ", ip("203.0.113.7")),
            Some(true)
        );
        // 单机写法等价于 /32
        assert_eq!(
            entry_matches("203.0.113.7/32", ip("203.0.113.7")),
            Some(true)
        );
    }

    #[test]
    fn test_entry_matches_cidr() {
        assert_eq!(entry_matches("10.0.0.0/8", ip("10.255.1.2")), Some(true));
        assert_eq!(entry_matches("10.0.0.0/8", ip("11.0.0.1")), Some(false));
        assert_eq!(entry_matches("0.0.0.0/0", ip("8.8.8.8")), Some(true));
        assert_eq!(
            entry_matches("192.168.1.0/24", ip("192.168.1.255")),
            Some(true)
        );
        assert_eq!(
            entry_matches("192.168.1.0/24", ip("192.168.2.1")),
            Some(false)
        );
    }

    #[test]
    fn test_entry_matches_malformed_is_none() {
        for bad in [
            "",
            "   ",
            "not-an-ip",
            "10.0.0.0/33",
            "10.0.0.0/",
            "10.0.0.0/-1",
            "10.0.0.0/8/8",
            "10.0.0.256",
            "2001:db8::1",
            "2001:db8::/32",
        ] {
            assert_eq!(
                entry_matches(bad, ip("10.0.0.1")),
                None,
                "应判为非法: {:?}",
                bad
            );
            assert!(!valid_ip_entry(bad), "应判为非法: {:?}", bad);
        }
        assert!(valid_ip_entry("10.0.0.0/8"));
        assert!(valid_ip_entry("127.0.0.1"));
    }
}

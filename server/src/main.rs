use axum::{extract::State, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions, SqliteRow};
use sqlx::Row;

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
    axum::serve(listener, app).await.unwrap();
}

// ===== 数据库 schema 与维护 =====

/// 当前 schema 版本，存于 `PRAGMA user_version`（该值随库文件持久化，不随连接）。
const SCHEMA_VERSION: i64 = 1;

/// 顺序迁移表：每项 (版本, 语句列表)。语句全部带 IF NOT EXISTS，对已有库幂等。
const MIGRATIONS: &[(i64, &[&str])] = &[(
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
)];

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

/// 注册应用（打包者调用）：存 app_id / secret / content_key / allow_temp
///
/// 防覆盖：app_id = 打包文件 MD5，任何拿到文件的人都能公开推导出来。
/// 若无校验，任何人都能 register 覆盖他人文件的 content_key / secret（使文件变砖或接管管理权）。
/// 因此：app_id 已存在时，只有持相同 secret 的同一打包者才允许更新，否则拒绝。
async fn register_app(State(db): State<Db>, Json(req): Json<RegisterRequest>) -> Json<ApiResponse> {
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
    sqlx::query("INSERT OR REPLACE INTO apps (app_id, secret, content_key, allow_temp, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&req.app_id)
        .bind(&req.secret)
        .bind(&req.content_key)
        .bind(allow_temp)
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
async fn get_key(State(db): State<Db>, Json(req): Json<KeyRequest>) -> Json<ApiResponse> {
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

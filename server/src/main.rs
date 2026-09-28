use axum::{
    extract::{ConnectInfo, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use pbkdf2::pbkdf2_hmac;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteRow,
};
use sqlx::Row;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::str::FromStr;
use std::time::Duration;

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

    // 连接参数一律走 SqliteConnectOptions 并交给「池」：池里每建立一条连接（含断线重连）
    // 都会带上这些设置。反过来，若在连接后单独执行一次 PRAGMA，只有执行它的那一条连接生效，
    // 池中其它连接仍然是默认行为——所以这里不那样做。
    let opts = SqliteConnectOptions::from_str(&db_url)
        .unwrap_or_else(|e| {
            // 解析失败属于部署配置错误，直接退出比带着半个配置启动更安全。
            eprintln!("DATABASE_URL 无法解析: {}", e);
            std::process::exit(1);
        })
        // WAL：读不阻塞写、写不阻塞读，写锁只作用于库文件的一部分而非整库。
        // 原实现沿用 SQLite 默认的 delete 日志模式，任何一次写都要独占整个库，
        // 并发请求下容易撞出 SQLITE_BUSY，正是数据库错误的主要来源。
        .journal_mode(SqliteJournalMode::Wal)
        // 忙等 5 秒：WAL 下写-写之间仍有互斥（含 checkpoint），5 秒足以覆盖正常写事务的排队，
        // 让短暂的争用表现为「稍等一下」而不是直接报错；再放长只会让真正的故障迟迟不返回，
        // 因此取与 sqlx 默认值一致的 5 秒。
        .busy_timeout(Duration::from_secs(5))
        // from_str 已解析 DATABASE_URL 的查询参数（如默认值里的 mode=rwc），
        // 这里再显式打开「文件不存在则创建」，保证 DATABASE_URL 不带 mode=rwc 时也照旧能建库。
        // 外键约束沿用 sqlx 默认的 ON（foreign_keys 未被覆盖），create/目录创建行为不变。
        .create_if_missing(true);

    let db = SqlitePoolOptions::new()
        .connect_with(opts)
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
    if let Err(e) = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    {
        // 服务循环本身出错时进程即将退出，打印原因后明确退出码，不再 panic。
        eprintln!("服务端异常退出: {}", e);
        std::process::exit(1);
    }
}

// ===== 数据库 schema 与维护 =====

/// 当前 schema 版本，存于 `PRAGMA user_version`（该值随库文件持久化，不随连接）。
const SCHEMA_VERSION: i64 = 3;

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
    // v3：grants / requests 增加用户口令散列列（pwd_salt = 16 字节随机盐的小写 hex，
    // pwd_hash = PBKDF2-HMAC-SHA256 导出 32 字节的小写 hex）。
    // 迁移前已存在的授权行 pwd_hash 为 NULL，/api/key 会拒绝并提示管理员重新授权——
    // 这是刻意的「默认安全」：旧数据没有口令就无法证明取钥人是谁。
    (
        3,
        &[
            "ALTER TABLE grants ADD COLUMN pwd_salt TEXT",
            "ALTER TABLE grants ADD COLUMN pwd_hash TEXT",
            "ALTER TABLE requests ADD COLUMN pwd_salt TEXT",
            "ALTER TABLE requests ADD COLUMN pwd_hash TEXT",
        ],
    ),
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
            // 加列语句没有 IF NOT EXISTS，单独做一次存在性判断（见 MIGRATIONS 中的说明）：
            // 从任意 `ALTER TABLE <表> ADD COLUMN <列> ...` 中取出表名与列名。
            if let Some(rest) = s.strip_prefix("ALTER TABLE ") {
                if let Some((table, col_def)) = rest.split_once(" ADD COLUMN ") {
                    let table = table.trim();
                    let col = col_def.split_whitespace().next().unwrap_or("");
                    if column_exists(db, table, col).await? {
                        continue;
                    }
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
    /// 该用户取钥时必须提供口令。缺失或为空一律拒绝授权。
    /// 用 Option 而非 String：缺字段也要走业务拒绝（success=false + 中文说明），
    /// 而不是 serde 的 422，便于调用方统一处理。
    #[serde(default)]
    password: Option<String>,
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
    /// 普通用户取钥时必须提供的口令（与 /api/grant 或 /api/approve 时设置的一致）
    #[serde(default)]
    password: Option<String>,
}

#[derive(Deserialize)]
struct AccessRequest {
    app_id: String,
    user_id: String,
    need_days: Option<i32>,
    message: Option<String>,
    /// 申请时由用户自己设定的口令，审批通过后随授权一起生效
    #[serde(default)]
    password: Option<String>,
}

#[derive(Deserialize)]
struct ApproveRequest {
    app_id: String,
    secret: String,
    user_id: String,
    expires_at: Option<String>,
    /// 管理员为该用户设定的口令；缺省时沿用申请时记录的口令散列
    #[serde(default)]
    password: Option<String>,
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

/// 处理函数里的内部故障（目前只有 SQLite 报错）。
///
/// 原实现里数据库错误一律 `.unwrap()`：任务 panic 后 axum 只能断开连接，
/// 调用方看到的是「连接被重置」，既与网络故障无法区分，也拿不到任何原因。
/// 现在统一转成 HTTP 500 + 与其它接口同形的 JSON（`success` / `message` 两个字段），
/// message 为固定中文文案，不含底层 SQL 细节。
struct ApiError(String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // 用 ApiResponse 而不是手写 json!：保证响应体形状与其它接口逐字一致。
        // 状态码用 500 而非 200：业务失败仍是 200 + success=false，
        // 这里表示服务端自身故障，调用方可据此区分「被业务拒绝」与「服务端坏了」。
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                message: self.0,
                key: None,
                requests: None,
            }),
        )
            .into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        // 细节只写服务端日志；返回调用方的是固定文案，避免泄露库结构、SQL 与文件路径。
        eprintln!("数据库操作失败: {}", e);
        ApiError("服务器内部错误：数据库操作失败，请稍后重试".into())
    }
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
async fn register_app(
    State(db): State<Db>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<ApiResponse>, ApiError> {
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
                return Ok(err(&format!(
                    "IP 白名单条目非法（仅支持 IPv4 单机或 CIDR，如 203.0.113.7、203.0.113.0/24）: {}",
                    bad
                )));
            }
            // 空数组与缺省等价，统一存 NULL
            if list.is_empty() {
                None
            } else {
                // 保留 unwrap：Vec<String> 序列化成 JSON 不会失败（键与值都是 String）。
                Some(serde_json::to_string(list).unwrap())
            }
        }
    };

    let existing: Option<SqliteRow> = sqlx::query("SELECT secret FROM apps WHERE app_id = ?")
        .bind(&req.app_id)
        .fetch_optional(&db)
        .await?;
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
            return Ok(err("该文件 ID 已被登记且密钥不匹配，拒绝覆盖"));
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
        .execute(&db)
        .await?;
    log_audit(&db, &req.app_id, "", "register", true, "注册成功").await;
    Ok(ok("应用已注册"))
}

/// 验证管理员密钥。数据库故障向上传播（由 ApiError 转成 JSON 500），不当作「密钥错误」。
async fn verify_secret(db: &Db, app_id: &str, secret: &str) -> Result<bool, sqlx::Error> {
    let row: Option<SqliteRow> =
        sqlx::query("SELECT 1 as x FROM apps WHERE app_id = ? AND secret = ?")
            .bind(app_id)
            .bind(secret)
            .fetch_optional(db)
            .await?;
    Ok(row.is_some())
}

/// 写审计日志（失败不阻断主流程）。
///
/// 刻意吞掉自己的错误：审计是旁路，写不进去也不能让业务请求失败或退化成 500。
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

// ===== 用户口令 =====

/// 口令散列参数：PBKDF2-HMAC-SHA256，10 万次迭代，导出 32 字节。
/// 盐是 16 字节随机数，以小写 hex 存库；派生结果同样以小写 hex 存库。
/// 这些常量属于服务端与调用方之间的约定，改动即破坏兼容。
const PBKDF2_ITERATIONS: u32 = 100_000;
const PWD_KEY_BYTES: usize = 32;

/// 缺省或空串都视为「未提供口令」。空口令不作数，否则等于没有口令。
fn provided_password(password: &Option<String>) -> Option<&str> {
    password.as_deref().filter(|p| !p.is_empty())
}

/// 字节序列转小写 hex
fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

/// hex 转字节；奇数长度或含非法字符时返回 None
fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in s.as_bytes().chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

/// 用给定盐（hex）派生口令散列（hex）
fn hash_password(password: &str, salt_hex: &str) -> String {
    // 盐无法解析时按空盐派生：结果必然与库里的散列不符，属于失败关闭，不会误放行。
    let salt = from_hex(salt_hex).unwrap_or_default();
    let mut out = [0u8; PWD_KEY_BYTES];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, PBKDF2_ITERATIONS, &mut out);
    to_hex(&out)
}

/// 生成新口令的 (盐, 散列)。
/// 盐由 SQLite 的 randomblob(16) 产生：服务端不为这一处再引入随机数依赖。
/// SQLite 的 hex() 返回大写，这里用 lower() 统一成协议约定的小写 hex。
async fn new_password_hash(db: &Db, password: &str) -> Result<(String, String), sqlx::Error> {
    let row: SqliteRow = sqlx::query("SELECT lower(hex(randomblob(16))) AS salt")
        .fetch_one(db)
        .await?;
    let salt: String = row.get("salt");
    let hash = hash_password(password, &salt);
    Ok((salt, hash))
}

/// 恒定时间比较两个 hex 散列：逐字节累积异或差，最后只判定一次。
/// 不用 `==`：字符串比较在首个不同字节处就会返回，泄露散列前缀的匹配进度。
/// 长度不同直接返回 false（散列长度固定，长度本身不是秘密）。
fn ct_eq_hex(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 校验口令：用存储的盐重新派生，再做恒定时间比较。盐或散列缺失、非法、不匹配一律 false。
fn verify_password(password: &str, salt_hex: &str, stored_hash: &str) -> bool {
    ct_eq_hex(&hash_password(password, salt_hex), stored_hash)
}

/// 授权用户（管理员）：必须同时设置该用户的口令，口令散列后随授权行一起存储。
async fn grant_user(
    State(db): State<Db>,
    Json(req): Json<GrantRequest>,
) -> Result<Json<ApiResponse>, ApiError> {
    if !verify_secret(&db, &req.app_id, &req.secret).await? {
        return Ok(err("密钥错误"));
    }
    // 没有口令的授权 = 谁填对 user_id 谁就能取钥，等同于没授权，直接拒绝。
    let Some(password) = provided_password(&req.password) else {
        log_audit(
            &db,
            &req.app_id,
            &req.user_id,
            "grant",
            false,
            "缺少用户口令",
        )
        .await;
        return Ok(err("缺少口令：授权时必须为该用户设置取钥口令"));
    };
    let expires = match req.expires_at.as_deref() {
        None => None,
        Some(input) => match parse_expire(input) {
            Ok(v) => v,
            Err(e) => {
                log_audit(&db, &req.app_id, &req.user_id, "grant", false, &e).await;
                return Ok(err(&e));
            }
        },
    };
    let (pwd_salt, pwd_hash) = new_password_hash(&db, password).await?;
    let now = chrono::Utc::now().format("%Y%m%d").to_string();
    sqlx::query("INSERT OR REPLACE INTO grants (app_id, user_id, granted_at, expires_at, pwd_salt, pwd_hash) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&req.app_id)
        .bind(&req.user_id)
        .bind(&now)
        .bind(expires)
        .bind(&pwd_salt)
        .bind(&pwd_hash)
        .execute(&db)
        .await?;
    log_audit(&db, &req.app_id, &req.user_id, "grant", true, "授权成功").await;
    Ok(ok(&format!("已授权 {}", req.user_id)))
}

/// 吊销用户（管理员）
async fn revoke_user(
    State(db): State<Db>,
    Json(req): Json<GrantRequest>,
) -> Result<Json<ApiResponse>, ApiError> {
    if !verify_secret(&db, &req.app_id, &req.secret).await? {
        return Ok(err("无权操作"));
    }
    sqlx::query("DELETE FROM grants WHERE app_id = ? AND user_id = ?")
        .bind(&req.app_id)
        .bind(&req.user_id)
        .execute(&db)
        .await?;
    log_audit(&db, &req.app_id, &req.user_id, "revoke", true, "吊销成功").await;
    Ok(ok(&format!("已吊销 {}", req.user_id)))
}

/// 获取解密密钥：管理员凭 secret 直接取；否则必须提供 user_id + 授权时设置的口令
///
/// 只有 user_id 不够：user_id 是请求体里的字符串，知道被授权人邮箱的人就能冒充。
/// 口令是「这个 user_id 确实是本人」的唯一凭据，因此没有口令、口令错误、
/// 或授权行本身没有口令散列（迁移前的老授权）都不下发密钥，并各记一条失败审计。
///
/// IP 白名单对管理员路径同样生效：白名单表达的是「这份密钥允许从哪里取」，
/// 而管理员持有 secret 本来就能重新 register 并改写/清空白名单，
/// 所以给管理员开绕行口子不增加任何防锁定能力，只会削弱限制本身，故两条路径统一校验。
async fn get_key(
    State(db): State<Db>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(req): Json<KeyRequest>,
) -> Result<Json<ApiResponse>, ApiError> {
    // 来源 IP 必须是 TCP 连接的对端地址（ConnectInfo），不接受请求体里自报的 IP：
    // 客户端可以随意伪造请求体字段，但对端地址由内核填充。
    let peer_ip = peer.ip();
    let app_id = req.app_id.clone();
    // 数据库故障用 ? 直接转 500；只有「IP 不匹配」这类业务拒绝才走下面的 err()。
    if let Err(reason) = check_source_ip(&db, &app_id, peer_ip).await? {
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
        return Ok(err(&format!("{}，拒绝下发密钥", reason)));
    }

    // 管理员：凭 secret 直接取密钥，无需申请/授权
    if let Some(secret) = &req.secret {
        if verify_secret(&db, &req.app_id, secret).await? {
            let app: Option<SqliteRow> =
                sqlx::query("SELECT content_key FROM apps WHERE app_id = ?")
                    .bind(&req.app_id)
                    .fetch_optional(&db)
                    .await?;
            return match app {
                Some(a) => {
                    let key: String = a.get("content_key");
                    log_audit(&db, &req.app_id, "admin", "key", true, "管理员取钥").await;
                    Ok(Json(ApiResponse {
                        success: true,
                        message: "管理员".into(),
                        key: Some(key),
                        requests: None,
                    }))
                }
                None => Ok(err("应用不存在")),
            };
        }
    }

    // 普通用户：查授权（expires_at 为空 = 永久）
    let Some(user_id) = &req.user_id else {
        return Ok(err("未授权，请申请临时权限或联系管理员"));
    };
    let now = chrono::Utc::now().format("%Y%m%d").to_string();

    // 查授权（连口令散列一起取出）
    let grant: Option<SqliteRow> = sqlx::query(
        "SELECT expires_at, pwd_salt, pwd_hash FROM grants WHERE app_id = ? AND user_id = ?",
    )
    .bind(&req.app_id)
    .bind(user_id)
    .fetch_optional(&db)
    .await?;

    let (pwd_salt, pwd_hash) = match grant {
        Some(g) => {
            let expires: Option<String> = g.get("expires_at");
            if let Some(exp) = &expires {
                if now > *exp {
                    sqlx::query("DELETE FROM grants WHERE app_id = ? AND user_id = ?")
                        .bind(&req.app_id)
                        .bind(user_id)
                        .execute(&db)
                        .await?;
                    log_audit(&db, &req.app_id, user_id, "key", false, "授权已过期").await;
                    return Ok(err(&format!("权限已过期（{}）", exp)));
                }
            }
            // expires 为空 → 永久有效
            (
                g.get::<Option<String>, _>("pwd_salt"),
                g.get::<Option<String>, _>("pwd_hash"),
            )
        }
        None => {
            log_audit(&db, &req.app_id, user_id, "key", false, "未授权").await;
            return Ok(err("未授权，请申请临时权限或联系管理员"));
        }
    };

    // 口令校验。先查「有没有口令」再比对口令：迁移前建立的授权 pwd_hash 为 NULL，
    // 无法证明取钥人就是被授权人，必须拒绝并让管理员重新授权（默认安全）。
    let (Some(pwd_salt), Some(pwd_hash)) = (pwd_salt, pwd_hash) else {
        log_audit(&db, &req.app_id, user_id, "key", false, "该授权未设置口令").await;
        return Ok(err("该授权未设置口令，请管理员重新授权"));
    };
    let Some(password) = provided_password(&req.password) else {
        log_audit(&db, &req.app_id, user_id, "key", false, "未提供口令").await;
        return Ok(err("未提供口令：请填写授权时设置的口令"));
    };
    if !verify_password(password, &pwd_salt, &pwd_hash) {
        log_audit(&db, &req.app_id, user_id, "key", false, "口令错误").await;
        return Ok(err("口令错误"));
    }

    // 下发 content_key
    let app: Option<SqliteRow> = sqlx::query("SELECT content_key FROM apps WHERE app_id = ?")
        .bind(&req.app_id)
        .fetch_optional(&db)
        .await?;

    match app {
        Some(a) => {
            let key: String = a.get("content_key");
            log_audit(&db, &req.app_id, user_id, "key", true, "下发密钥").await;
            Ok(Json(ApiResponse {
                success: true,
                message: "成功".into(),
                key: Some(key),
                requests: None,
            }))
        }
        None => Ok(err("应用不存在")),
    }
}

/// 申请临时权限（用户）：必须自带口令，且申请天数必须是正整数。
async fn request_access(
    State(db): State<Db>,
    Json(req): Json<AccessRequest>,
) -> Result<Json<ApiResponse>, ApiError> {
    // 口令与天数先校验：这是申请的两项必备条件，缺一项就拒绝，避免写出没有口令的申请。
    let Some(password) = provided_password(&req.password) else {
        return Ok(err(
            "缺少口令：申请时必须设置口令，审批通过后用该口令取密钥",
        ));
    };
    let Some(need_days) = req.need_days.filter(|d| *d > 0) else {
        return Ok(err("申请天数必须为正整数（need_days 至少为 1）"));
    };

    // 已授权则无需申请
    let existing: Option<SqliteRow> =
        sqlx::query("SELECT 1 as x FROM grants WHERE app_id = ? AND user_id = ?")
            .bind(&req.app_id)
            .bind(&req.user_id)
            .fetch_optional(&db)
            .await?;
    if existing.is_some() {
        return Ok(ok("已有权限，直接打开即可"));
    }

    // 检查是否开启临时申请
    let app: Option<SqliteRow> = sqlx::query("SELECT allow_temp FROM apps WHERE app_id = ?")
        .bind(&req.app_id)
        .fetch_optional(&db)
        .await?;
    match app {
        Some(a) => {
            let at: Option<i64> = a.get("allow_temp");
            if at.unwrap_or(0) == 0 {
                return Ok(err("该文件未开启临时权限申请"));
            }
        }
        None => return Ok(err("应用不存在")),
    }

    // 避免重复待审批
    let pending: Option<SqliteRow> = sqlx::query(
        "SELECT 1 as x FROM requests WHERE app_id = ? AND user_id = ? AND status = 'pending'",
    )
    .bind(&req.app_id)
    .bind(&req.user_id)
    .fetch_optional(&db)
    .await?;
    if pending.is_some() {
        return Ok(ok("已有待审批申请，请耐心等待"));
    }

    let now = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let (pwd_salt, pwd_hash) = new_password_hash(&db, password).await?;
    sqlx::query("INSERT INTO requests (app_id, user_id, need_days, message, status, created_at, pwd_salt, pwd_hash) VALUES (?, ?, ?, ?, 'pending', ?, ?, ?)")
        .bind(&req.app_id).bind(&req.user_id)
        .bind(need_days).bind(&req.message).bind(&now)
        .bind(&pwd_salt).bind(&pwd_hash)
        .execute(&db).await?;
    log_audit(
        &db,
        &req.app_id,
        &req.user_id,
        "request",
        true,
        "提交临时权限申请",
    )
    .await;
    Ok(ok("已提交申请，请等待管理员审批"))
}

/// 待审批列表（管理员查看：需该文件的 secret，且只返回本文件的申请）
async fn list_requests(
    State(db): State<Db>,
    Json(req): Json<ListRequestsRequest>,
) -> Result<Json<ApiResponse>, ApiError> {
    if !verify_secret(&db, &req.app_id, &req.secret).await? {
        return Ok(err("无权查看（密钥错误）"));
    }
    let rows = sqlx::query("SELECT user_id, need_days, message, created_at FROM requests WHERE app_id = ? AND status = 'pending' ORDER BY created_at DESC")
        .bind(&req.app_id)
        .fetch_all(&db).await?;

    let requests: Vec<RequestItem> = rows
        .iter()
        .map(|r| RequestItem {
            user_id: r.get("user_id"),
            need_days: r.get("need_days"),
            message: r.get("message"),
            created_at: r.get("created_at"),
        })
        .collect();

    Ok(Json(ApiResponse {
        success: true,
        message: "成功".into(),
        key: None,
        requests: Some(requests),
    }))
}

/// 审计日志（管理员查看：需该文件的 secret）
///
/// 与 /api/healthz 一样返回 `Json<serde_json::Value>`（多一个 logs 字段），
/// 因此这里返回 `Result<Json<serde_json::Value>, ApiError>`：错误类型与其它接口共用，
/// 只是把成功分支包进 Result，是本次唯一为错误处理改动的返回类型。
async fn list_logs(
    State(db): State<Db>,
    Json(req): Json<LogsRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !verify_secret(&db, &req.app_id, &req.secret).await? {
        return Ok(Json(
            serde_json::json!({ "success": false, "message": "无权查看（密钥错误）" }),
        ));
    }
    let rows = sqlx::query("SELECT user_id, action, success, details, created_at FROM audit_logs WHERE app_id = ? ORDER BY id DESC LIMIT 200")
        .bind(&req.app_id)
        .fetch_all(&db).await?;
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
    Ok(Json(
        serde_json::json!({ "success": true, "message": "成功", "logs": logs }),
    ))
}

/// 审批通过（管理员）：必须有待审批申请，且最终一定带口令写入授权。
async fn approve_request(
    State(db): State<Db>,
    Json(req): Json<ApproveRequest>,
) -> Result<Json<ApiResponse>, ApiError> {
    if !verify_secret(&db, &req.app_id, &req.secret).await? {
        return Ok(err("无权操作"));
    }

    // 先确认确实有待审批申请：否则「审批通过」就成了凭空造授权（旧实现会直接插 grants 行）。
    let request: Option<SqliteRow> = sqlx::query(
        "SELECT need_days, pwd_salt, pwd_hash FROM requests WHERE app_id = ? AND user_id = ? AND status = 'pending'",
    )
    .bind(&req.app_id)
    .bind(&req.user_id)
    .fetch_optional(&db)
    .await?;

    let Some(request) = request else {
        log_audit(
            &db,
            &req.app_id,
            &req.user_id,
            "approve",
            false,
            "没有待审批的申请",
        )
        .await;
        return Ok(err("没有待审批的申请，拒绝审批"));
    };

    let need_days: Option<i32> = request.get("need_days");

    // 口令来源：管理员指定优先（等于管理员替用户设口令），否则沿用申请时存下的散列。
    // 两者都没有就拒绝——绝不允许落到「授权没有口令」这种状态。
    let (pwd_salt, pwd_hash) = match provided_password(&req.password) {
        Some(password) => {
            let (salt, hash) = new_password_hash(&db, password).await?;
            (salt, hash)
        }
        None => {
            let salt: Option<String> = request.get("pwd_salt");
            let hash: Option<String> = request.get("pwd_hash");
            match (salt, hash) {
                (Some(salt), Some(hash)) => (salt, hash),
                _ => {
                    log_audit(
                        &db,
                        &req.app_id,
                        &req.user_id,
                        "approve",
                        false,
                        "申请未带口令且管理员未指定口令",
                    )
                    .await;
                    return Ok(err(
                        "该申请没有口令：请在审批时用 password 指定，或让用户重新申请",
                    ));
                }
            }
        }
    };

    // 计算过期时间：优先用管理员指定，否则按申请天数，再否则永久
    let expires = match req.expires_at.as_deref() {
        None => None,
        Some(input) => match parse_expire(input) {
            Ok(v) => v,
            Err(e) => {
                log_audit(&db, &req.app_id, &req.user_id, "approve", false, &e).await;
                return Ok(err(&e));
            }
        },
    }
    .or_else(|| need_days.map(|d| days_from_now(d as i64)));

    let now = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    sqlx::query("UPDATE requests SET status = 'approved', resolved_at = ? WHERE app_id = ? AND user_id = ? AND status = 'pending'")
        .bind(&now).bind(&req.app_id).bind(&req.user_id)
        .execute(&db).await?;

    let granted = chrono::Utc::now().format("%Y%m%d").to_string();
    sqlx::query("INSERT OR REPLACE INTO grants (app_id, user_id, granted_at, expires_at, pwd_salt, pwd_hash) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&req.app_id).bind(&req.user_id).bind(&granted).bind(&expires)
        .bind(&pwd_salt).bind(&pwd_hash)
        .execute(&db).await?;

    log_audit(&db, &req.app_id, &req.user_id, "approve", true, "审批通过").await;
    Ok(ok(&format!("已通过 {}", req.user_id)))
}

/// 审批拒绝（管理员）
async fn deny_request(
    State(db): State<Db>,
    Json(req): Json<ApproveRequest>,
) -> Result<Json<ApiResponse>, ApiError> {
    if !verify_secret(&db, &req.app_id, &req.secret).await? {
        return Ok(err("无权操作"));
    }
    let now = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    sqlx::query("UPDATE requests SET status = 'denied', resolved_at = ? WHERE app_id = ? AND user_id = ? AND status = 'pending'")
        .bind(&now).bind(&req.app_id).bind(&req.user_id)
        .execute(&db).await?;
    log_audit(&db, &req.app_id, &req.user_id, "deny", true, "审批拒绝").await;
    Ok(ok(&format!("已拒绝 {}", req.user_id)))
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
/// 返回 `Ok(None)` 表示不限制（列为 NULL 或空串、或存的是空数组）；
/// 返回 `Ok(Some(条目))` 表示限制开启，其中条目可能为空（表示库里存了无法解析的内容）；
/// 返回 `Err` 表示数据库故障，由调用方转成 JSON 500（不能当作「不限制」放行）。
async fn load_whitelist(db: &Db, app_id: &str) -> Result<Option<Vec<String>>, sqlx::Error> {
    let row: Option<SqliteRow> = sqlx::query("SELECT ip_whitelist FROM apps WHERE app_id = ?")
        .bind(app_id)
        .fetch_optional(db)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let raw: Option<String> = row.get("ip_whitelist");
    let raw = match raw {
        None => return Ok(None),
        Some(s) if s.trim().is_empty() => return Ok(None),
        Some(s) => s,
    };
    match serde_json::from_str::<Vec<String>>(&raw) {
        Ok(v) if v.is_empty() => Ok(None),
        Ok(v) => Ok(Some(v)),
        // 库里存了无法解析的内容（正常流程写不进来，只可能被人工改动）：
        // 打印告警并失败关闭，返回空条目集 → 所有来源都不匹配 → 拒绝下发密钥。
        Err(e) => {
            eprintln!(
                "应用 {} 的 ip_whitelist 无法解析（{}），按拒绝处理: {}",
                app_id, e, raw
            );
            Ok(Some(Vec::new()))
        }
    }
}

/// 取钥前的来源 IP 校验。
///
/// 返回类型是嵌套的 Result：外层 `Err` 是数据库故障（由 `?` 转成 JSON 500），
/// 内层 `Ok(())` 放行、`Ok(Err(原因))` 是业务拒绝。两者混在一起会把「库读不出来」
/// 说成「IP 不在白名单」，因此必须分开。
///
/// 语义：
/// - 未设白名单（NULL / 空数组）→ 不限制，任何来源都放行，老部署行为不变；
/// - IPv4 来源命中任一条目 → 放行；
/// - IPv4 来源未命中 → 拒绝；
/// - 条目非法（含 IPv6）→ 忽略该条目并告警；若因此没有任何可用条目，则全部拒绝（失败关闭）；
/// - IPv6 来源地址 → 本服务端不实现 IPv6 匹配，白名单非空时一律拒绝（失败关闭）。
async fn check_source_ip(
    db: &Db,
    app_id: &str,
    ip: IpAddr,
) -> Result<Result<(), String>, sqlx::Error> {
    let Some(entries) = load_whitelist(db, app_id).await? else {
        return Ok(Ok(()));
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
        Ok(Ok(()))
    } else {
        Ok(Err(format!("来源 IP {} 不在该文件的 IP 白名单内", ip)))
    }
}

/// 解析过期时间，只接受三类写法，其余一律 Err（由调用方拒绝请求）：
/// - 空串 / `永久` / `永久有效` / `permanent`（大小写不敏感）→ `Ok(None)`，无过期；
/// - `Nd` / `ND`（天数必须为正整数）→ `Ok(Some(YYYYMMDD))`；
/// - 恰好 8 位 ASCII 数字 `YYYYMMDD` → `Ok(Some(原值))`。
///
/// 旧实现把兜底分支写成 `Some(input.to_string())`，任意垃圾都被当成过期日期存库，
/// 之后按字符串大小比较，等于用一个无法解释的值决定授权生死，所以改为显式拒绝。
fn parse_expire(input: &str) -> Result<Option<String>, String> {
    let input = input.trim();
    // 空 / 永久 / permanent → 无过期（永久授权）
    if input.is_empty()
        || input == "永久"
        || input == "永久有效"
        || input.eq_ignore_ascii_case("permanent")
    {
        return Ok(None);
    }
    if let Some(days) = input.strip_suffix('d').or_else(|| input.strip_suffix('D')) {
        let n: i64 = days
            .parse()
            .map_err(|_| format!("过期时间格式非法（支持 Nd、YYYYMMDD、永久）: {}", input))?;
        if n <= 0 {
            return Err(format!("过期天数必须为正整数: {}", input));
        }
        return Ok(Some(days_from_now(n)));
    }
    if input.len() == 8 && input.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(Some(input.to_string()));
    }
    Err(format!(
        "过期时间格式非法（支持 Nd、YYYYMMDD、永久）: {}",
        input
    ))
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

#[cfg(test)]
mod expire_tests {
    use super::parse_expire;

    #[test]
    fn test_parse_expire_accepts_known_forms() {
        // 无过期
        assert_eq!(parse_expire(""), Ok(None));
        assert_eq!(parse_expire("   "), Ok(None));
        assert_eq!(parse_expire("永久"), Ok(None));
        assert_eq!(parse_expire("永久有效"), Ok(None));
        assert_eq!(parse_expire("permanent"), Ok(None));
        assert_eq!(parse_expire("PERMANENT"), Ok(None));
        // 天数：必须换算成 YYYYMMDD
        let v = parse_expire("30d").unwrap().unwrap();
        assert_eq!(v.len(), 8, "天数应换算成 YYYYMMDD: {}", v);
        assert!(v.bytes().all(|b| b.is_ascii_digit()));
        assert!(parse_expire("1D").unwrap().is_some());
        // 8 位日期原样保留
        assert_eq!(parse_expire("20301231"), Ok(Some("20301231".into())));
        assert_eq!(parse_expire(" 20301231 "), Ok(Some("20301231".into())));
    }

    #[test]
    fn test_parse_expire_rejects_junk() {
        for bad in [
            "junk",
            "not-a-date",
            "2025-01-01",
            "2025/01/01",
            "2025010",
            "202501011",
            "2025010a",
            "20250101x",
            "0d",
            "-1d",
            "d",
            "abc d",
            "1.5d",
            "２０２５０１０１",
        ] {
            assert!(
                parse_expire(bad).is_err(),
                "非法写法必须被拒绝，不能当日期存库: {:?}",
                bad
            );
        }
    }
}

#[cfg(test)]
mod password_tests {
    use super::{ct_eq_hex, hash_password, verify_password, PBKDF2_ITERATIONS};

    /// 已知答案向量：由 `hashlib.pbkdf2_hmac('sha256', b'hunter2', bytes.fromhex(salt), 100000, 32)`
    /// 独立算出。CLI / GUI 侧的口令派生必须复现同一结果，否则取钥必然失败。
    #[test]
    fn test_hash_password_known_answer() {
        let salt = "00112233445566778899aabbccddeeff";
        assert_eq!(
            hash_password("hunter2", salt),
            "2147d14169e0af3690f720de3f98dd89d63e4dbb08a702ebbda7b33f0fe97a69"
        );
    }

    #[test]
    fn test_hash_password_is_stable_and_hex() {
        let salt = "00112233445566778899aabbccddeeff";
        let h1 = hash_password("hunter2", salt);
        let h2 = hash_password("hunter2", salt);
        assert_eq!(h1, h2, "同盐同口令必须得到同一散列");
        assert_eq!(h1.len(), 64, "32 字节应编码为 64 位 hex");
        assert!(h1
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        // 换盐或换口令都必须变
        assert_ne!(
            h1,
            hash_password("hunter2", "ffeeddccbbaa99887766554433221100")
        );
        assert_ne!(h1, hash_password("hunter3", salt));
    }

    #[test]
    fn test_verify_password_only_accepts_exact_match() {
        let salt = "0123456789abcdef0123456789abcdef";
        let stored = hash_password("正确口令", salt);
        assert!(verify_password("正确口令", salt, &stored));
        assert!(!verify_password("错误口令", salt, &stored));
        assert!(!verify_password("", salt, &stored));
        // 非法盐 / 非法散列一律失败关闭
        assert!(!verify_password("正确口令", "zz", &stored));
        assert!(!verify_password("正确口令", salt, ""));
        assert!(!verify_password("正确口令", salt, &stored.to_uppercase()));
    }

    #[test]
    fn test_ct_eq_hex() {
        assert!(ct_eq_hex("00ff", "00ff"));
        assert!(!ct_eq_hex("00ff", "00fe"));
        assert!(!ct_eq_hex("00ff", "00ff00"));
        assert!(!ct_eq_hex("", "00"));
        assert!(ct_eq_hex("", ""));
    }

    #[test]
    fn test_pbkdf2_iterations_is_100000() {
        assert_eq!(PBKDF2_ITERATIONS, 100_000);
    }
}

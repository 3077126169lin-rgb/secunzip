use std::path::{Path, PathBuf};
use crate::core::{PackConfig, KeyNode, KeySource, RunMode, AuthMode, OutputFormat, CompressAlgo, CryptoAlgo, HashAlgo};
use crate::packer::PackBuilder;
use crate::runtime::RuntimeLoader;
use crate::Result;

/// 打包命令
/// 打包结果（供 CLI 与 GUI 共用）
pub struct PackOutcome {
    /// 文件 ID = 打包文件的 MD5
    pub app_id: String,
    /// 管理密钥
    pub secret: String,
    /// 是否成功注册到服务端（未注册则无法联网打开）
    pub registered: bool,
}

/// 执行打包：生成/采用密钥 → 加密打包 → 注册服务端（托管 content_key）→ 保存管理密钥
/// custom_key：管理员自定义密钥（留空则随机生成）
pub fn pack_file(sources: Vec<PathBuf>, output: PathBuf, server: String, allow_temp: bool, custom_key: Option<String>, blackbox: bool) -> Result<PackOutcome> {
    for source in &sources {
        if !source.exists() {
            return Err(crate::SecUnzipError::Packing(format!("路径不存在: {}", source.display())));
        }
    }

    let secret = uuid::Uuid::new_v4().to_string();
    // content_key：文件加密密钥，只交给服务端托管，服务端仅向授权用户下发
    let content_key = custom_key.filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string() + &uuid::Uuid::new_v4().to_string());

    let config = PackConfig {
        format: if blackbox { OutputFormat::Exe } else { OutputFormat::SecUnzip },
        compress: CompressAlgo::Zip,
        crypto: CryptoAlgo::Aes256Gcm,
        hash: HashAlgo::Sha256,
        key_derive: KeyNode::Input(KeySource::Literal(content_key.clone())),
        run_mode: RunMode::TempDir,
        auth_mode: AuthMode::Remote(server.clone()),
        expire_at: None,
        ip_whitelist: Vec::new(),
        salt: rand::random::<[u8; 32]>().to_vec(),
        app_id: None, // 文件 ID 用打包后文件的 MD5，不写入头部
        allow_temp,
    };

    let builder = PackBuilder::new(config, sources, output.clone());
    builder.build()?;

    // 文件 ID = 打包后文件的 MD5（内容寻址，免手动生成）
    let app_id = crate::crypto::file_md5_hex(&output)?;

    // 注册到服务器（文件ID=MD5，托管 content_key）
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registered = rt.block_on(async {
        register_app(&server, &app_id, &secret, &content_key, allow_temp).await
    }).is_ok();

    if !registered {
        // 注册失败 → 清理产物，绝不留下无法联网打开的「死文件」
        let _ = std::fs::remove_file(&output);
        return Err(crate::SecUnzipError::Packing(
            "注册服务端失败，已清理打包产物（不留无法打开的死文件）。请检查服务端地址/网络后重试".into(),
        ));
    }

    // 保存 secret 到本地文件
    save_secret(&output, &secret)?;

    Ok(PackOutcome { app_id, secret, registered })
}

/// 打包命令（CLI，打印结果）
pub fn cmd_pack(sources: Vec<PathBuf>, output: PathBuf, server: String, allow_temp: bool) -> Result<()> {
    let out = pack_file(sources, output.clone(), server.clone(), allow_temp, None, false)?;

    println!();
    println!("═══════════════════════════════════════════════════════════");
    println!("打包完成: {}", output.display());
    println!("═══════════════════════════════════════════════════════════");
    println!();
    println!("管理信息（请妥善保管）:");
    println!("文件ID:    {}  (打包文件的 MD5)", out.app_id);
    println!("管理密钥:  {}", out.secret);
    println!("服务端:    {}", server);
    if !out.registered {
        println!("注册服务端失败，文件暂无法联网打开");
    }
    if allow_temp {
        println!("临时申请:   已开启");
    }
    println!();
    println!("授权用户:");
    println!("secunzip grant {} --user <用户ID> [--expires 7d]", output.display());
    println!();
    println!("用户申请:");
    println!("secunzip request {} --user <用户ID> --days 3", output.display());
    println!();
    println!("查看申请:");
    println!("secunzip requests {}", output.display());
    println!("═══════════════════════════════════════════════════════════");

    Ok(())
}

/// 打开命令
pub fn cmd_open(file: PathBuf, user: String, output: Option<PathBuf>) -> Result<()> {
    let loader = RuntimeLoader::from_file(&file)?;
    let header = loader.header();

    match &header.config.auth_mode {
        AuthMode::Remote(server) => {
            println!("联网验证...");
            println!("用户: {}", user);

            let app_id = get_app_id(&file)?;

            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
                request_key(server, &app_id, &user).await
            });

            match result {
                Ok(key) => {
                    println!("验证通过，获取到解密密钥");
                    match output {
                        Some(dest) => {
                            // 指定输出目录：解压落盘
                            let files = match loader.extract_with_key(&key)? {
                                crate::runtime::loader::ExtractResult::Files(f) => f,
                                crate::runtime::loader::ExtractResult::Executed => {
                                    println!("执行完成");
                                    return Ok(());
                                }
                            };
                            std::fs::create_dir_all(&dest)?;
                            for (name, content) in files {
                                let p = dest.join(&name);
                                if let Some(parent) = p.parent() { std::fs::create_dir_all(parent)?; }
                                std::fs::write(p, content)?;
                            }
                            println!("解压完成: {}", dest.display());
                        }
                        None => {
                            // 无输出：内存挂载，只读浏览（不落盘）
                            let vfs = loader.mount_vfs_with_key(&key)?;
                            println!();
                            println!("═══════════════════════════════════════════════════════════");
                            println!("内存挂载（只读，未落盘）  文件数: {}  大小: {}", vfs.file_count(), human_size(vfs.total_size()));
                            println!("═══════════════════════════════════════════════════════════");
                            print_tree(&vfs, "", "");
                            println!("═══════════════════════════════════════════════════════════");
                            println!("加 --output <目录> 可解压到磁盘");
                        }
                    }
                }
                Err(e) => {
                    println!("{}", e);
                    println!("提示: 如果没有权限，可以申请临时权限:");
                    println!("secunzip request {} --user {} --days 3", file.display(), user);
                    return Err(crate::SecUnzipError::Other(e));
                }
            }
        }
        AuthMode::Local => {
            return Err(crate::SecUnzipError::Other("本地模式不支持".into()));
        }
    }
    Ok(())
}

/// 打印内存文件树（虚拟挂载效果）
fn print_tree(vfs: &crate::runtime::VirtualFS, dir: &str, prefix: &str) {
    let entries = vfs.list_dir(dir);
    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == entries.len() - 1;
        let branch = if is_last { "└── " } else { "├── " };
        if entry.is_dir {
            println!("{}{} {}/", prefix, branch, entry.name);
            let child_prefix = format!("{}{}", prefix, if is_last { "    " } else { "│   " });
            print_tree(vfs, &entry.path, &child_prefix);
        } else {
            println!("{}{} {}  ({})", prefix, branch, entry.name, human_size(entry.size));
        }
    }
}

/// 人类可读大小
fn human_size(size: usize) -> String {
    if size < 1024 { format!("{} B", size) }
    else if size < 1024 * 1024 { format!("{:.1} KB", size as f64 / 1024.0) }
    else { format!("{:.1} MB", size as f64 / (1024.0 * 1024.0)) }
}

/// 授权命令
pub fn cmd_grant(file: PathBuf, user: String, expires: Option<String>) -> Result<()> {
    let app_id = get_app_id(&file)?;
    let secret = get_secret(&file)?;
    let server = get_server(&file)?;

    println!("授权用户 {}...", user);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        grant_user(&server, &app_id, &secret, &user, expires.as_deref()).await
    });

    match result {
        Ok(msg) => println!("{}", msg),
        Err(e) => println!("{}", e),
    }
    Ok(())
}

/// 吊销命令
pub fn cmd_revoke(file: PathBuf, user: String) -> Result<()> {
    let app_id = get_app_id(&file)?;
    let secret = get_secret(&file)?;
    let server = get_server(&file)?;

    println!("吊销用户 {}...", user);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        revoke_user(&server, &app_id, &secret, &user).await
    });

    match result {
        Ok(msg) => println!("{}", msg),
        Err(e) => println!("{}", e),
    }
    Ok(())
}

/// 申请临时权限
pub fn cmd_request(file: PathBuf, user: String, days: Option<i32>, message: Option<String>) -> Result<()> {
    let app_id = get_app_id(&file)?;
    let server = get_server(&file)?;

    println!("申请临时权限...");

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        request_access(&server, &app_id, &user, days, message).await
    });

    match result {
        Ok(msg) => println!("{}", msg),
        Err(e) => println!("{}", e),
    }
    Ok(())
}

/// 查看待审批
pub fn cmd_requests(file: PathBuf) -> Result<()> {
    let app_id = get_app_id(&file)?;
    let secret = get_secret(&file)?;
    let server = get_server(&file)?;

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        list_requests(&server, &app_id, &secret).await
    });

    match result {
        Ok(requests) => {
            if requests.is_empty() {
                println!("没有待审批的申请");
            } else {
                println!("待审批列表:");
                println!();
                for r in requests {
                    println!("用户: {}", r.user_id);
                    if let Some(days) = r.need_days {
                        println!("需要: {}天", days);
                    }
                    if let Some(msg) = &r.message {
                        println!("说明: {}", msg);
                    }
                    println!("时间: {}", r.created_at);
                    println!("---");
                }
            }
        }
        Err(e) => println!("{}", e),
    }
    Ok(())
}

/// 审批通过
pub fn cmd_approve(file: PathBuf, user: String, expires: Option<String>) -> Result<()> {
    let app_id = get_app_id(&file)?;
    let secret = get_secret(&file)?;
    let server = get_server(&file)?;

    println!("审批通过 {}...", user);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        approve_request(&server, &app_id, &secret, &user, expires.as_deref()).await
    });

    match result {
        Ok(msg) => println!("{}", msg),
        Err(e) => println!("{}", e),
    }
    Ok(())
}

/// 审批拒绝
pub fn cmd_deny(file: PathBuf, user: String) -> Result<()> {
    let app_id = get_app_id(&file)?;
    let secret = get_secret(&file)?;
    let server = get_server(&file)?;

    println!("审批拒绝 {}...", user);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        deny_request(&server, &app_id, &secret, &user).await
    });

    match result {
        Ok(msg) => println!("{}", msg),
        Err(e) => println!("{}", e),
    }
    Ok(())
}

// ===== HTTP 请求 =====

async fn register_app(server: &str, app_id: &str, secret: &str, content_key: &str, allow_temp: bool) -> std::result::Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client.post(format!("{}/api/register", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "content_key": content_key,
            "allow_temp": allow_temp,
        }))
        .send().await.map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("响应解析失败: {}", e))?;
    if data["success"].as_bool().unwrap_or(false) {
        Ok(())
    } else {
        Err(data["message"].as_str().unwrap_or("注册失败").to_string())
    }
}

async fn request_key(server: &str, app_id: &str, user_id: &str) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client.post(format!("{}/api/key", server))
        .json(&serde_json::json!({ "app_id": app_id, "user_id": user_id }))
        .send().await.map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("响应解析失败: {}", e))?;

    if data["success"].as_bool().unwrap_or(false) {
        Ok(data["key"].as_str().unwrap_or("").to_string())
    } else {
        Err(data["message"].as_str().unwrap_or("未知错误").to_string())
    }
}

async fn grant_user(server: &str, app_id: &str, secret: &str, user_id: &str, expires_at: Option<&str>) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client.post(format!("{}/api/grant", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "user_id": user_id,
            "expires_at": expires_at,
        }))
        .send().await.map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("响应解析失败: {}", e))?;
    Ok(data["message"].as_str().unwrap_or("").to_string())
}

async fn revoke_user(server: &str, app_id: &str, secret: &str, user_id: &str) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client.post(format!("{}/api/revoke", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "user_id": user_id,
        }))
        .send().await.map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("响应解析失败: {}", e))?;
    Ok(data["message"].as_str().unwrap_or("").to_string())
}

async fn request_access(server: &str, app_id: &str, user_id: &str, need_days: Option<i32>, message: Option<String>) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client.post(format!("{}/api/request", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "user_id": user_id,
            "need_days": need_days,
            "message": message,
        }))
        .send().await.map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("响应解析失败: {}", e))?;
    Ok(data["message"].as_str().unwrap_or("").to_string())
}

#[derive(serde::Deserialize)]
struct RequestInfo {
    user_id: String,
    need_days: Option<i32>,
    message: Option<String>,
    created_at: String,
}

async fn list_requests(server: &str, app_id: &str, secret: &str) -> std::result::Result<Vec<RequestInfo>, String> {
    let client = reqwest::Client::new();
    let resp = client.post(format!("{}/api/requests", server))
        .json(&serde_json::json!({ "app_id": app_id, "secret": secret }))
        .send().await.map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("响应解析失败: {}", e))?;

    if data["success"].as_bool().unwrap_or(false) {
        let requests: Vec<RequestInfo> = serde_json::from_value(data["requests"].clone())
            .map_err(|e| format!("解析失败: {}", e))?;
        Ok(requests)
    } else {
        Err(data["message"].as_str().unwrap_or("未知错误").to_string())
    }
}

async fn approve_request(server: &str, app_id: &str, secret: &str, user_id: &str, expires_at: Option<&str>) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client.post(format!("{}/api/approve", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "user_id": user_id,
            "expires_at": expires_at,
        }))
        .send().await.map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("响应解析失败: {}", e))?;
    Ok(data["message"].as_str().unwrap_or("").to_string())
}

async fn deny_request(server: &str, app_id: &str, secret: &str, user_id: &str) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client.post(format!("{}/api/deny", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "user_id": user_id,
        }))
        .send().await.map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("响应解析失败: {}", e))?;
    Ok(data["message"].as_str().unwrap_or("").to_string())
}

// ===== 工具函数 =====

/// 保存 secret 到本地文件
fn save_secret(output: &Path, secret: &str) -> Result<()> {
    let secret_path = output.with_extension("secret");
    std::fs::write(&secret_path, secret)?;
    println!("管理密钥已保存: {}", secret_path.display());
    Ok(())
}

/// 文件 ID = 文件内容的 MD5（内容寻址）
fn get_app_id(file: &Path) -> Result<String> {
    crate::crypto::file_md5_hex(file)
}

/// 从本地文件读取 secret
fn get_secret(file: &Path) -> Result<String> {
    let secret_path = file.with_extension("secret");
    if secret_path.exists() {
        Ok(std::fs::read_to_string(secret_path)?.trim().to_string())
    } else {
        // 尝试环境变量
        std::env::var("SECUNZIP_SECRET")
            .map_err(|_| crate::SecUnzipError::Other(
                format!("未找到密钥文件 {}，请设置 SECUNZIP_SECRET 环境变量", secret_path.display())
            ))
    }
}

/// 从文件头部读取 server 地址
fn get_server(file: &Path) -> Result<String> {
    let loader = crate::runtime::RuntimeLoader::from_file(file)?;
    let header = loader.header();
    
    match &header.config.auth_mode {
        crate::core::AuthMode::Remote(url) => Ok(url.clone()),
        _ => Err(crate::SecUnzipError::Other("文件未配置服务端地址".into())),
    }
}

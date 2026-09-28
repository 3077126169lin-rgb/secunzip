use crate::core::{
    AuthMode, CompressAlgo, CryptoAlgo, HashAlgo, KeyNode, KeySource, OutputFormat, PackConfig,
    RunMode,
};
use crate::packer::PackBuilder;
use crate::runtime::RuntimeLoader;
use crate::Result;
use std::path::{Path, PathBuf};

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
pub fn pack_file(
    sources: Vec<PathBuf>,
    output: PathBuf,
    server: String,
    allow_temp: bool,
    custom_key: Option<String>,
    blackbox: bool,
) -> Result<PackOutcome> {
    for source in &sources {
        if !source.exists() {
            return Err(crate::SecUnzipError::Packing(format!(
                "路径不存在: {}",
                source.display()
            )));
        }
    }

    let secret = uuid::Uuid::new_v4().to_string();
    // content_key：文件加密密钥，只交给服务端托管，服务端仅向授权用户下发
    let content_key = custom_key
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string() + &uuid::Uuid::new_v4().to_string());

    let config = pack_config(server.clone(), content_key.clone(), allow_temp, blackbox);

    let builder = PackBuilder::new(config, sources, output.clone());
    builder.build()?;

    // 文件 ID = 打包后文件的 MD5（内容寻址，免手动生成）
    let app_id = crate::crypto::file_md5_hex(&output)?;

    // 注册到服务器（文件ID=MD5，托管 content_key）
    let rt = tokio::runtime::Runtime::new().unwrap();
    let registered = rt
        .block_on(async { register_app(&server, &app_id, &secret, &content_key, allow_temp).await })
        .is_ok();

    if !registered {
        // 注册失败 → 清理产物，绝不留下无法联网打开的「死文件」
        let _ = std::fs::remove_file(&output);
        return Err(crate::SecUnzipError::Packing(
            "注册服务端失败，已清理打包产物（不留无法打开的死文件）。请检查服务端地址/网络后重试"
                .into(),
        ));
    }

    // 保存 secret 到本地文件
    save_secret(&output, &secret)?;

    Ok(PackOutcome {
        app_id,
        secret,
        registered,
    })
}

/// 按 CLI 选项构造打包配置
///
/// blackbox = true 时产物为 EXE 格式（[runner][内嵌 .secunzip][尾部标记]），双击可自解压运行；
/// 否则为自有 .secunzip 格式。
pub fn pack_config(
    server: String,
    content_key: String,
    allow_temp: bool,
    blackbox: bool,
) -> PackConfig {
    PackConfig {
        format: if blackbox {
            OutputFormat::Exe
        } else {
            OutputFormat::SecUnzip
        },
        compress: CompressAlgo::Zip,
        crypto: CryptoAlgo::Aes256Gcm,
        hash: HashAlgo::Sha256,
        key_derive: KeyNode::Input(KeySource::Literal(content_key)),
        run_mode: RunMode::TempDir,
        auth_mode: AuthMode::Remote(server),
        expire_at: None,
        ip_whitelist: Vec::new(),
        salt: rand::random::<[u8; 32]>().to_vec(),
        app_id: None, // 文件 ID 用打包后文件的 MD5，不写入头部
        allow_temp,
    }
}

/// 打包命令（CLI，打印结果）
pub fn cmd_pack(
    sources: Vec<PathBuf>,
    output: PathBuf,
    server: String,
    allow_temp: bool,
    blackbox: bool,
) -> Result<()> {
    let out = pack_file(
        sources,
        output.clone(),
        server.clone(),
        allow_temp,
        None,
        blackbox,
    )?;

    println!();
    println!("═══════════════════════════════════════════════════════════");
    println!("打包完成: {}", output.display());
    println!("═══════════════════════════════════════════════════════════");
    println!();
    println!("管理信息（请妥善保管）:");
    println!("文件ID:    {}  (打包文件的 MD5)", out.app_id);
    println!("管理密钥:  {}", out.secret);
    println!("服务端:    {}", server);
    if blackbox {
        println!("产物类型:  黑盒自解压 EXE（双击运行；用户标识取自 --user / SECUNZIP_USER / 客户端配置）");
    }
    if !out.registered {
        println!("注册服务端失败，文件暂无法联网打开");
    }
    if allow_temp {
        println!("临时申请:   已开启");
    }
    println!();
    println!("授权用户:");
    println!(
        "secunzip grant {} --user <用户ID> --password <口令> [--expires 7d]",
        output.display()
    );
    println!();
    println!("用户申请:");
    println!(
        "secunzip request {} --user <用户ID> --days 3 --password <口令>",
        output.display()
    );
    println!();
    println!("查看申请:");
    println!("secunzip requests {}", output.display());
    println!("═══════════════════════════════════════════════════════════");

    Ok(())
}

/// 打开命令
pub fn cmd_open(
    file: PathBuf,
    user: String,
    output: Option<PathBuf>,
    password: Option<String>,
    remember: bool,
) -> Result<()> {
    let loader = RuntimeLoader::from_file(&file)?;
    let app_id = get_app_id(&file)?;
    open_remote(
        &loader,
        &app_id,
        &user,
        output,
        &file.display().to_string(),
        password,
        remember,
    )
}

/// 黑盒自解压 EXE 入口
///
/// runner（打包工具自身）启动时自查尾部标记，命中后由 `main` 调用本函数：
/// 取出内嵌 .secunzip → 内存载入 → 走与 `open` 相同的联网取密钥流程。
/// 文件 ID 用 EXE 自身的 MD5，与打包时 `pack_file` 的算法一致。
pub fn cmd_blackbox(
    exe_path: &Path,
    exe: &[u8],
    user: Option<String>,
    output: Option<PathBuf>,
    password: Option<String>,
    remember: bool,
) -> Result<()> {
    let embedded = crate::runtime::embedded_secunzip(exe)?;
    let loader = RuntimeLoader::from_bytes(embedded)?;
    let app_id = crate::crypto::md5_hex(exe);
    let user = resolve_blackbox_user(user)?;
    println!("黑盒自解压 EXE: {}", exe_path.display());
    open_remote(
        &loader,
        &app_id,
        &user,
        output,
        &exe_path.display().to_string(),
        password,
        remember,
    )
}

/// 联网取密钥并打开：CLI `open`（从文件）与黑盒 EXE（从内存）共用
///
/// hint_target 仅用于失败提示里拼出「申请临时权限」的命令行。
/// password 为命令行显式口令，缺省时依次回退到 SECUNZIP_PASSWORD、配置记住的口令、标准输入一行。
/// remember 为真时，只在成功取到密钥之后才把口令记入本地配置。
#[allow(clippy::too_many_arguments)]
fn open_remote(
    loader: &RuntimeLoader,
    app_id: &str,
    user: &str,
    output: Option<PathBuf>,
    hint_target: &str,
    password: Option<String>,
    remember: bool,
) -> Result<()> {
    let server = match &loader.header().config.auth_mode {
        AuthMode::Remote(server) => server.clone(),
        AuthMode::Local => {
            return Err(crate::SecUnzipError::Other("本地模式不支持".into()));
        }
    };

    // 口令由服务端校验，客户端只负责明文传输：缺失时在本地就报错，不发空口令
    let ctx = PasswordContext {
        app_id,
        server: &server,
        user,
    };
    let password = resolve_password(password, &ctx)?;

    println!("联网验证...");
    println!("用户: {}", user);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async { request_key(&server, app_id, user, &password.value).await });

    match result {
        Ok(key) => {
            println!("验证通过，获取到解密密钥");
            // 取密钥成功才记住口令；口令本来就是从配置里读出来的就不必重写
            if remember && password.source != PasswordSource::Config {
                match remember_user_password(app_id, &server, user, &password.value) {
                    Ok(()) => println!("口令已记入本地配置"),
                    Err(e) => println!("提示: 口令未能记入本地配置（{}）", e),
                }
            }
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
                        if let Some(parent) = p.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::write(p, content)?;
                    }
                    println!("解压完成: {}", dest.display());
                }
                None => {
                    // 无输出：内存挂载，只读浏览（不落盘）
                    let vfs = loader.mount_vfs_with_key(&key)?;
                    println!();
                    println!("═══════════════════════════════════════════════════════════");
                    println!(
                        "内存挂载（只读，未落盘）  文件数: {}  大小: {}",
                        vfs.file_count(),
                        human_size(vfs.total_size())
                    );
                    println!("═══════════════════════════════════════════════════════════");
                    print_tree(&vfs, "", "");
                    println!("═══════════════════════════════════════════════════════════");
                    println!("加 --output <目录> 可解压到磁盘");
                }
            }
        }
        Err(e) => {
            println!("{}", e);
            // 存过的口令被拒：多半是管理员改过口令，给出可操作的下一步而不是只报错
            if password.source == PasswordSource::Config {
                println!("提示: 上面用的是本地配置里记住的口令，可能已被管理员修改。");
                println!("请用 --password <新口令> 重新打开；同时加 --remember 可更新记住的口令");
            }
            println!("提示: 如果没有权限，可以申请临时权限:");
            println!(
                "secunzip request {} --user {} --days 3 --password <口令>",
                hint_target, user
            );
            return Err(crate::SecUnzipError::Other(e));
        }
    }
    Ok(())
}

/// 黑盒模式下解析用户标识：`--user` > 环境变量 SECUNZIP_USER > 客户端配置 config.json
///
/// 双击运行的黑盒 EXE 没有命令行参数，因此需要回退到本机已登录的客户端配置。
fn resolve_blackbox_user(explicit: Option<String>) -> Result<String> {
    let from_arg = explicit
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    if let Some(u) = from_arg {
        return Ok(u);
    }
    let from_env = std::env::var("SECUNZIP_USER")
        .ok()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    if let Some(u) = from_env {
        return Ok(u);
    }
    if let Some(u) = config_user_id() {
        return Ok(u);
    }
    Err(crate::SecUnzipError::Other(
        "未找到用户标识。请用「黑盒EXE --user <用户ID>」运行，或设置 SECUNZIP_USER 环境变量，\
         或先用客户端设置个人 ID（写入 %USERPROFILE%\\Documents\\SecUnzip\\config.json）"
            .into(),
    ))
}

/// 口令来源规范化：空串/纯空白视为未提供
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

/// 口令来源：失败提示要区分「来自配置」与「临时输入」
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordSource {
    /// 命令行 `--password`
    Arg,
    /// 环境变量 `SECUNZIP_PASSWORD`
    Env,
    /// 配置文件里记住的口令
    Config,
    /// 标准输入一行
    Stdin,
}

/// 解析出的口令：明文 + 来源
#[derive(Debug, Clone)]
pub struct ResolvedPassword {
    pub value: String,
    pub source: PasswordSource,
}

/// 取密钥口令的匹配上下文（文件 ID + 服务器 + 用户）
pub struct PasswordContext<'a> {
    pub app_id: &'a str,
    pub server: &'a str,
    pub user: &'a str,
}

/// 口令选取（纯函数，便于测试）：参数 > 环境变量 > 配置记住的口令 > 标准输入
///
/// 逐级回退，空串/纯空白不算提供；全部缺失时报中文错误而不下发空口令。
pub fn pick_password(
    explicit: Option<String>,
    from_env: Option<String>,
    from_config: Option<String>,
    from_stdin: Option<String>,
) -> Result<ResolvedPassword> {
    let candidates = [
        (explicit, PasswordSource::Arg),
        (from_env, PasswordSource::Env),
        (from_config, PasswordSource::Config),
        (from_stdin, PasswordSource::Stdin),
    ];
    for (value, source) in candidates {
        if let Some(value) = non_empty(value) {
            return Ok(ResolvedPassword { value, source });
        }
    }
    Err(crate::SecUnzipError::Other(
        "缺少口令：请用 --password <口令> 指定，或设置 SECUNZIP_PASSWORD 环境变量，\
         或在标准输入提供一行口令"
            .into(),
    ))
}

/// 可选口令选取（纯函数）：参数 > 环境变量；都没有则 None（不下发该字段）
///
/// 刻意不含「配置里记住的口令」：审核类操作留空有明确语义（沿用申请时的口令），
/// 静默套用一个记住的口令会把申请者自己设的口令覆盖掉。
pub fn pick_optional_password(
    explicit: Option<String>,
    from_env: Option<String>,
) -> Option<ResolvedPassword> {
    [
        (explicit, PasswordSource::Arg),
        (from_env, PasswordSource::Env),
    ]
    .into_iter()
    .find_map(|(value, source)| non_empty(value).map(|value| ResolvedPassword { value, source }))
}

/// 从标准输入读取一行口令（去掉行尾换行；EOF/读失败视为未提供）
fn read_password_line() -> Option<String> {
    use std::io::BufRead;
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_string()),
        Err(_) => None,
    }
}

/// 取密钥口令（open / request）：参数 > 环境变量 > 配置 > 标准输入一行
///
/// 前三者有值时不再读标准输入，避免交互场景下白白吞掉一行输入。
pub fn resolve_password(
    explicit: Option<String>,
    ctx: &PasswordContext,
) -> Result<ResolvedPassword> {
    let from_env = std::env::var("SECUNZIP_PASSWORD").ok();
    let from_config = config_user_password(ctx.app_id, ctx.server, ctx.user);
    if non_empty(explicit.clone()).is_some()
        || non_empty(from_env.clone()).is_some()
        || non_empty(from_config.clone()).is_some()
    {
        return pick_password(explicit, from_env, from_config, None);
    }
    pick_password(explicit, from_env, from_config, read_password_line())
}

/// 管理口令（grant / approve，必填）：参数 > 环境变量 > 配置 > 标准输入一行
pub fn resolve_admin_password(explicit: Option<String>) -> Result<ResolvedPassword> {
    let from_env = std::env::var("SECUNZIP_PASSWORD").ok();
    let from_config = config_admin_password();
    if non_empty(explicit.clone()).is_some()
        || non_empty(from_env.clone()).is_some()
        || non_empty(from_config.clone()).is_some()
    {
        return pick_password(explicit, from_env, from_config, None);
    }
    pick_password(explicit, from_env, from_config, read_password_line())
}

/// 审批口令（approve 可选）：**只认本次显式输入** —— 参数 > 环境变量
///
/// 两者都没有时返回 None，整个字段不下发，由服务端沿用申请者申请时设定的口令。
/// 刻意不回退到配置里记住的管理口令：否则 `grant --remember` 之后的任何一次
/// `approve -u alice` 都会静默覆盖掉申请者自己设的口令。
/// 也不读标准输入：审批常常是批量执行，阻塞等待输入是错误行为。
pub fn resolve_optional_admin_password(explicit: Option<String>) -> Option<ResolvedPassword> {
    let from_env = std::env::var("SECUNZIP_PASSWORD").ok();
    pick_optional_password(explicit, from_env)
}

// ===== 本地配置文件（与 GUI 共用同一份 config.json） =====

/// 配置文件候选路径：`Documents\SecUnzip\config.json`，其次旧的 `%APPDATA%\SecUnzip\config.json`
fn config_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        paths.push(
            PathBuf::from(home)
                .join("Documents")
                .join("SecUnzip")
                .join("config.json"),
        );
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        paths.push(PathBuf::from(appdata).join("SecUnzip").join("config.json"));
    }
    paths
}

/// 读取现有配置（路径 + 内容）；解析失败视为不存在，绝不覆盖无法解析的文件
fn load_config() -> Option<(PathBuf, serde_json::Value)> {
    for path in config_paths() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                return Some((path, value));
            }
        }
    }
    None
}

/// 读取客户端配置文件里的 user_id（与 GUI 共用同一份配置，只读，不改写）
fn config_user_id() -> Option<String> {
    let (_, value) = load_config()?;
    value["user_id"]
        .as_str()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
}

/// 查找记住的取密钥口令（与 GUI 的 remembered_passwords 同结构同规则）：
/// 先按「文件 ID + 用户」精确匹配，再退到「服务器 + 用户」
fn config_user_password(app_id: &str, server: &str, user: &str) -> Option<String> {
    let (_, value) = load_config()?;
    let entries = value["remembered_passwords"].as_array()?;
    let field = |e: &serde_json::Value, key: &str| e[key].as_str().unwrap_or("").to_string();
    let usable =
        |e: &serde_json::Value| !field(e, "password").is_empty() && field(e, "user_id") == user;
    // 1. 文件 + 用户精确匹配
    let exact = entries
        .iter()
        .find(|e| usable(e) && field(e, "app_id") == app_id);
    // 2. 服务器 + 用户回退（黑盒 EXE 的 app_id 与 .secunzip 不同，靠这条命中）
    let fallback = entries
        .iter()
        .find(|e| usable(e) && field(e, "server_url") == server);
    exact.or(fallback).map(|e| field(e, "password"))
}

/// 读取记住的管理口令（grant/approve 专用字段，与取密钥口令分开存放）
fn config_admin_password() -> Option<String> {
    let (_, value) = load_config()?;
    non_empty(value["admin_password"].as_str().map(|s| s.to_string()))
}

/// 写回配置：保留其它字段；文件不存在时新建一份（GUI 能正常读入）
fn save_config(update: impl FnOnce(&mut serde_json::Value)) -> Result<()> {
    let (path, mut value) = match load_config() {
        Some((path, value)) => (path, value),
        None => {
            let path = config_paths().into_iter().next().ok_or_else(|| {
                crate::SecUnzipError::Other("找不到用户目录，无法写入配置".into())
            })?;
            // 存在但读不出来（坏 JSON）：宁可记住失败，也不能覆盖用户原有配置
            if path.exists() {
                return Err(crate::SecUnzipError::Other(
                    "配置文件无法解析（不是合法 JSON），为免覆盖未做改动".into(),
                ));
            }
            (path, serde_json::json!({ "user_id": "", "server_url": "" }))
        }
    };
    if !value.is_object() {
        return Err(crate::SecUnzipError::Other(
            "配置文件格式异常（不是 JSON 对象），未改动".into(),
        ));
    }
    update(&mut value);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&value)
        .map_err(|e| crate::SecUnzipError::Other(format!("配置序列化失败: {}", e)))?;
    std::fs::write(&path, text)?;
    Ok(())
}

/// 记住取密钥口令：与 GUI 的 remembered_passwords 同结构，GUI 保存时不会丢
///
/// 同「文件 + 用户」的旧记录先删后插，保持最近使用在前，最多保留 32 条。
pub fn remember_user_password(
    app_id: &str,
    server: &str,
    user: &str,
    password: &str,
) -> Result<()> {
    if app_id.trim().is_empty() || user.trim().is_empty() || password.trim().is_empty() {
        return Ok(());
    }
    save_config(|value| {
        if value["user_id"].as_str().unwrap_or("").trim().is_empty() {
            value["user_id"] = serde_json::Value::String(user.to_string());
        }
        if value["server_url"].as_str().unwrap_or("").trim().is_empty() {
            value["server_url"] = serde_json::Value::String(server.to_string());
        }
        let list = value["remembered_passwords"]
            .as_array_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        let mut list: Vec<serde_json::Value> = list
            .into_iter()
            .filter(|e| {
                !(e["app_id"].as_str().unwrap_or("") == app_id
                    && e["user_id"].as_str().unwrap_or("") == user)
            })
            .collect();
        list.insert(
            0,
            serde_json::json!({
                "app_id": app_id,
                "server_url": server,
                "user_id": user,
                "password": password,
            }),
        );
        list.truncate(32);
        value["remembered_passwords"] = serde_json::Value::Array(list);
    })
}

/// 管理口令（grant/approve 专用字段，绝不与取密钥口令混存）
pub fn remember_admin_password(server: &str, password: &str) -> Result<()> {
    if password.trim().is_empty() {
        return Ok(());
    }
    save_config(|value| {
        if value["server_url"].as_str().unwrap_or("").trim().is_empty() && !server.is_empty() {
            value["server_url"] = serde_json::Value::String(server.to_string());
        }
        value["admin_password"] = serde_json::Value::String(password.to_string());
    })
}

/// 管理操作成功后的记住动作：只在成功之后调用；写配置失败只提示，不影响已完成的授权/审批
///
/// 只有本次**显式输入**的口令才会被记住（approve 的 source 只可能是 Arg/Env，
/// open/request 会从配置里读到 Config，那种情况本来就在配置里，不必重写）。
fn remember_admin_after_success(password: &ResolvedPassword, server: &str, remember: bool) {
    if !remember || password.source == PasswordSource::Config {
        return;
    }
    match remember_admin_password(server, &password.value) {
        Ok(()) => println!("口令已记入本地配置"),
        Err(e) => println!("提示: 口令未能记入本地配置（{}）", e),
    }
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
            println!(
                "{}{} {}  ({})",
                prefix,
                branch,
                entry.name,
                human_size(entry.size)
            );
        }
    }
}

/// 人类可读大小
fn human_size(size: usize) -> String {
    if size < 1024 {
        format!("{} B", size)
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    }
}

/// 授权命令
pub fn cmd_grant(
    file: PathBuf,
    user: String,
    expires: Option<String>,
    password: Option<String>,
    remember: bool,
) -> Result<()> {
    let app_id = get_app_id(&file)?;
    let secret = get_secret(&file)?;
    let server = get_server(&file)?;
    // 服务端要求该字段：口令缺失/为空时在本地报错，不发空口令
    let password = resolve_admin_password(password)?;

    println!("授权用户 {}...", user);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        grant_user(
            &server,
            &app_id,
            &secret,
            &user,
            expires.as_deref(),
            &password.value,
        )
        .await
    });

    match result {
        Ok(msg) => {
            println!("{}", msg);
            remember_admin_after_success(&password, &server, remember);
        }
        Err(e) => return Err(crate::SecUnzipError::Other(e)),
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
    let result = rt.block_on(async { revoke_user(&server, &app_id, &secret, &user).await });

    match result {
        Ok(msg) => println!("{}", msg),
        Err(e) => return Err(crate::SecUnzipError::Other(e)),
    }
    Ok(())
}

/// 申请天数校验：只接受正整数（None 表示不指定天数）
pub fn validate_need_days(days: Option<i32>) -> Result<()> {
    match days {
        Some(d) if d <= 0 => Err(crate::SecUnzipError::Other(format!(
            "申请天数必须是正整数（当前 {}），请用 --days <正整数>",
            d
        ))),
        _ => Ok(()),
    }
}

/// 申请临时权限
pub fn cmd_request(
    file: PathBuf,
    user: String,
    days: Option<i32>,
    message: Option<String>,
    password: Option<String>,
    remember: bool,
) -> Result<()> {
    // 申请天数只接受正整数，0/负数在本地拦下
    validate_need_days(days)?;
    let app_id = get_app_id(&file)?;
    let server = get_server(&file)?;
    let ctx = PasswordContext {
        app_id: &app_id,
        server: &server,
        user: &user,
    };
    let password = resolve_password(password, &ctx)?;

    println!("申请临时权限...");

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        request_access(&server, &app_id, &user, days, message, &password.value).await
    });

    match result {
        Ok(msg) => {
            println!("{}", msg);
            // 申请成功才记住：之后审批通过即可直接用同一口令取密钥
            if remember && password.source != PasswordSource::Config {
                match remember_user_password(&app_id, &server, &user, &password.value) {
                    Ok(()) => println!("口令已记入本地配置"),
                    Err(e) => println!("提示: 口令未能记入本地配置（{}）", e),
                }
            }
        }
        Err(e) => return Err(crate::SecUnzipError::Other(e)),
    }
    Ok(())
}

/// 查看待审批
pub fn cmd_requests(file: PathBuf) -> Result<()> {
    let app_id = get_app_id(&file)?;
    let secret = get_secret(&file)?;
    let server = get_server(&file)?;

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async { list_requests(&server, &app_id, &secret).await });

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
        Err(e) => return Err(crate::SecUnzipError::Other(e)),
    }
    Ok(())
}

/// 审批通过
///
/// 口令留空 = 沿用申请者申请时设定的口令：此时不下发 password 字段，
/// 由服务端从申请行沿用。只有本次显式给出（`--password` / `SECUNZIP_PASSWORD`）
/// 才覆盖，并且此时 `--remember` 才会把它记入配置供以后的 grant 使用。
pub fn cmd_approve(
    file: PathBuf,
    user: String,
    expires: Option<String>,
    password: Option<String>,
    remember: bool,
) -> Result<()> {
    let app_id = get_app_id(&file)?;
    let secret = get_secret(&file)?;
    let server = get_server(&file)?;
    let password = resolve_optional_admin_password(password);
    if password.is_none() {
        println!("未指定口令，沿用申请者设定的口令");
    }

    println!("审批通过 {}...", user);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        approve_request(
            &server,
            &app_id,
            &secret,
            &user,
            expires.as_deref(),
            password.as_ref().map(|p| p.value.as_str()),
        )
        .await
    });

    match result {
        Ok(msg) => {
            println!("{}", msg);
            if let Some(password) = &password {
                remember_admin_after_success(password, &server, remember);
            }
        }
        Err(e) => return Err(crate::SecUnzipError::Other(e)),
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
    let result = rt.block_on(async { deny_request(&server, &app_id, &secret, &user).await });

    match result {
        Ok(msg) => println!("{}", msg),
        Err(e) => return Err(crate::SecUnzipError::Other(e)),
    }
    Ok(())
}

// ===== HTTP 请求 =====

async fn register_app(
    server: &str,
    app_id: &str,
    secret: &str,
    content_key: &str,
    allow_temp: bool,
) -> std::result::Result<(), String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/register", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "content_key": content_key,
            "allow_temp": allow_temp,
        }))
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;
    if data["success"].as_bool().unwrap_or(false) {
        Ok(())
    } else {
        Err(data["message"].as_str().unwrap_or("注册失败").to_string())
    }
}

async fn request_key(
    server: &str,
    app_id: &str,
    user_id: &str,
    password: &str,
) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/key", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "user_id": user_id,
            "password": password,
        }))
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;

    if data["success"].as_bool().unwrap_or(false) {
        Ok(data["key"].as_str().unwrap_or("").to_string())
    } else {
        // 口令错误 / 缺口令 / 该授权未设口令等拒绝理由，原样透出服务端 message
        Err(data["message"].as_str().unwrap_or("未知错误").to_string())
    }
}

/// 服务端响应统一判定：success=false 一律返回 Err(message)，绝不把拒绝当成功
fn response_result(
    data: &serde_json::Value,
    fallback: &str,
) -> std::result::Result<String, String> {
    if data["success"].as_bool().unwrap_or(false) {
        Ok(data["message"].as_str().unwrap_or("").to_string())
    } else {
        Err(data["message"].as_str().unwrap_or(fallback).to_string())
    }
}

async fn grant_user(
    server: &str,
    app_id: &str,
    secret: &str,
    user_id: &str,
    expires_at: Option<&str>,
    password: &str,
) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/grant", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "user_id": user_id,
            "expires_at": expires_at,
            "password": password,
        }))
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;
    response_result(&data, "授权失败")
}

async fn revoke_user(
    server: &str,
    app_id: &str,
    secret: &str,
    user_id: &str,
) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/revoke", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "user_id": user_id,
        }))
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;
    response_result(&data, "吊销失败")
}

async fn request_access(
    server: &str,
    app_id: &str,
    user_id: &str,
    need_days: Option<i32>,
    message: Option<String>,
    password: &str,
) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    // need_days 只在正整数时下发（未指定则整个字段不发），0/负数已在 cmd_request 拦下
    let mut body = serde_json::json!({
        "app_id": app_id,
        "user_id": user_id,
        "message": message,
        "password": password,
    });
    if let Some(d) = need_days {
        body["need_days"] = serde_json::Value::from(d);
    }
    let resp = client
        .post(format!("{}/api/request", server))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;
    response_result(&data, "申请失败")
}

#[derive(serde::Deserialize)]
struct RequestInfo {
    user_id: String,
    need_days: Option<i32>,
    message: Option<String>,
    created_at: String,
}

async fn list_requests(
    server: &str,
    app_id: &str,
    secret: &str,
) -> std::result::Result<Vec<RequestInfo>, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/requests", server))
        .json(&serde_json::json!({ "app_id": app_id, "secret": secret }))
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;

    if data["success"].as_bool().unwrap_or(false) {
        let requests: Vec<RequestInfo> = serde_json::from_value(data["requests"].clone())
            .map_err(|e| format!("解析失败: {}", e))?;
        Ok(requests)
    } else {
        Err(data["message"].as_str().unwrap_or("未知错误").to_string())
    }
}

async fn approve_request(
    server: &str,
    app_id: &str,
    secret: &str,
    user_id: &str,
    expires_at: Option<&str>,
    password: Option<&str>,
) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    // 未提供口令时整个字段都不下发（不是空串），由服务端沿用申请行里的口令
    let mut body = serde_json::json!({
        "app_id": app_id,
        "secret": secret,
        "user_id": user_id,
        "expires_at": expires_at,
    });
    if let Some(p) = password {
        body["password"] = serde_json::Value::String(p.to_string());
    }
    let resp = client
        .post(format!("{}/api/approve", server))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;
    response_result(&data, "审批失败")
}

async fn deny_request(
    server: &str,
    app_id: &str,
    secret: &str,
    user_id: &str,
) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/deny", server))
        .json(&serde_json::json!({
            "app_id": app_id,
            "secret": secret,
            "user_id": user_id,
        }))
        .send()
        .await
        .map_err(|e| format!("网络错误: {}", e))?;

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {}", e))?;
    response_result(&data, "拒绝失败")
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
        std::env::var("SECUNZIP_SECRET").map_err(|_| {
            crate::SecUnzipError::Other(format!(
                "未找到密钥文件 {}，请设置 SECUNZIP_SECRET 环境变量",
                secret_path.display()
            ))
        })
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

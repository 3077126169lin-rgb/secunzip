//! 自解压运行时骨架（虚拟字节占位，打包时替换）
//!
//! 结构: [EXE 存根][PackHeader][加密数据]
//! 运行时解析头部、联网验证、请求密钥、内存解压浏览。
//! 仅支持临时权限申请（无预授权/管理密钥），用户 ID 自动取本地用户名。

use crate::Result;

/// 自解压运行时骨架（虚拟字节占位，打包时替换）
pub const RUNTIME_SKELETON: &[u8] = &[0x4D, 0x5A, 0x90, 0x00];

/// 解析并执行自解压流程
#[allow(dead_code)]
fn parse_and_execute(exe_data: &[u8], header_bytes: &[u8], encrypted: &[u8]) -> Result<()> {
    use crate::core::PackHeader;

    // 解析头部
    let _header = PackHeader::from_bytes(header_bytes)?;

    // 解密解压
    let loader = crate::runtime::RuntimeLoader::from_bytes(exe_data)?;
    let _result = loader.extract()?;

    Ok(())
}

/// 简易 GUI 事件（抽象，实际可接 egui/iced/slint 等）
#[allow(dead_code)]
enum Input {
    /// 设置申请天数
    RequestDays(String),
    /// 申请临时权限
    ApplyTemp,
    /// 检查状态并打开（已审批通过则打开）
    CheckOpen,
    /// 退出
    Quit,
}

/// 应用状态
#[allow(dead_code)]
struct State {
    app_id: String,
    server_url: String,
    /// 本地用户名（自动获取，无需手填）
    username: String,
    /// 申请天数
    request_days: String,
    /// 状态消息
    status: String,
    status_is_error: bool,
    /// 已挂载的内存文件树（虚拟挂载）
    mounted: Option<crate::runtime::VirtualFS>,
    pending_action: Option<Input>,
}

#[allow(dead_code)]
impl State {
    /// 处理用户输入
    fn handle_input(&mut self, input: Input) {
        match input {
            Input::RequestDays(text) => self.request_days = text,
            Input::Quit => std::process::exit(0),
            Input::ApplyTemp => {
                // 仅临时申请：用本地用户名提交申请
                let days: i32 = self.request_days.parse().unwrap_or(7);
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    request_access(&self.server_url, &self.app_id, &self.username, Some(days), None).await
                });
                match result {
                    Ok(msg) => self.show_status(&format!("{}", msg), false),
                    Err(e) => self.show_status(&format!("{}", e), true),
                }
            }
            Input::CheckOpen => {
                // 检查是否已审批通过 → 获取密钥 → 内存挂载
                self.show_status("正在连接服务器...", false);
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result = rt.block_on(async {
                    request_key(&self.server_url, &self.app_id, &self.username).await
                });
                match result {
                    Ok(key) => {
                        match mount_files(&self.app_id, &key) {
                            Ok(vfs) => {
                                self.show_status(&format!(
                                    "已通过（用户: {}）· 内存挂载 {} 个文件",
                                    self.username, vfs.file_count()
                                ), false);
                                self.mounted = Some(vfs);
                            }
                            Err(e) => self.show_status(&format!("解密失败: {}", e), true),
                        }
                    }
                    Err(e) => {
                        self.show_status(&format!("{}", e), true);
                    }
                }
            }
        }
    }

    fn show_status(&mut self, msg: &str, is_error: bool) {
        self.status = msg.to_string();
        self.status_is_error = is_error;
    }

    /// 渲染界面（抽象，实际接 GUI 框架）
    fn show(&mut self) {
        println!("\n SecUnzip - 受控内容");
        println!("──────────────────────────────────");
        println!("应用ID:  {}", &self.app_id[..8.min(self.app_id.len())]);
        println!("服务器:  {}", self.server_url);
        println!("用户:    {}（本地用户名，自动）", self.username);
        println!("申请天数: [{}]", self.request_days);
        println!();
        println!("[ 申请临时权限]   [ 检查并打开]   [0 退出]");
        if !self.status.is_empty() {
            println!("状态: {}", self.status);
        }
        println!("──────────────────────────────────");

        // 已挂载则显示文件树
        if let Some(vfs) = &self.mounted {
            println!("内存挂载（只读，未落盘）");
            show_file_tree(vfs, "", "");
        }
    }
}

/// 打开文件（内存浏览，不落盘）
#[allow(dead_code)]
fn open_file_in_memory(exe_data: &[u8]) -> Result<()> {
    use crate::runtime::RuntimeLoader;

    let loader = RuntimeLoader::from_bytes(exe_data)?;
    match loader.extract()? {
        crate::runtime::loader::ExtractResult::Files(files) => {
            println!("文件列表（内存中）:");
            for (name, content) in &files {
                println!("{}  ({} B)", name, content.len());
            }
            Ok(())
        }
        crate::runtime::loader::ExtractResult::Executed => Ok(()),
    }
}

/// 用服务端密钥解密并挂载内存文件系统
#[allow(dead_code)]
fn mount_files(exe_data: &[u8], key: &str) -> Result<crate::runtime::VirtualFS> {
    use crate::runtime::RuntimeLoader;
    let loader = RuntimeLoader::from_bytes(exe_data)?;
    loader.mount_vfs_with_key(key)
}

/// 打印内存文件树
#[allow(dead_code)]
fn show_file_tree(vfs: &crate::runtime::VirtualFS, dir: &str, prefix: &str) {
    let entries = vfs.list_dir(dir);
    for (i, entry) in entries.iter().enumerate() {
        let is_last = i == entries.len() - 1;
        let branch = if is_last { "└── " } else { "├── " };
        if entry.is_dir {
            println!("{}{} {}/", prefix, branch, entry.name);
            let child = format!("{}{}", prefix, if is_last { "    " } else { "│   " });
            show_file_tree(vfs, &entry.path, &child);
        } else {
            println!("{}{} {}  ({} B)", prefix, branch, entry.name, entry.size);
        }
    }
}

/// 获取本地用户名（当前登录用户）
#[allow(dead_code)]
fn get_local_username() -> String {
    // Windows: USERNAME 环境变量
    if let Ok(u) = std::env::var("USERNAME") {
        if !u.trim().is_empty() {
            return u.trim().to_string();
        }
    }
    // Unix: USER
    if let Ok(u) = std::env::var("USER") {
        if !u.trim().is_empty() {
            return u.trim().to_string();
        }
    }
    // whoami 兜底（Windows 输出 domain\user，取最后一段）
    if let Ok(out) = std::process::Command::new("whoami").output() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return s.rsplit('\\').next().unwrap_or(&s).to_string();
        }
    }
    hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

/// 创建初始状态
#[allow(dead_code)]
fn default_state(app_id: String, server_url: String) -> State {
    State {
        app_id,
        server_url,
        username: get_local_username(),
        request_days: "7".into(),
        status: "本程序仅支持临时权限申请".into(),
        status_is_error: false,
        mounted: None,
        pending_action: None,
    }
}

// ===== 网络请求（与服务端通信） =====

#[allow(dead_code)]
async fn request_key(server: &str, app_id: &str, user_id: &str) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client.post(&format!("{}/api/key", server))
        .json(&serde_json::json!({ "app_id": app_id, "user_id": user_id }))
        .send().await.map_err(|e| format!("网络错误: {}", e))?;
    let data: serde_json::Value = resp.json().await.map_err(|e| format!("响应解析失败: {}", e))?;
    if data["success"].as_bool().unwrap_or(false) {
        Ok(data["key"].as_str().unwrap_or("").to_string())
    } else {
        Err(data["message"].as_str().unwrap_or("未知错误").to_string())
    }
}

#[allow(dead_code)]
async fn request_access(server: &str, app_id: &str, user_id: &str, need_days: Option<i32>, message: Option<String>) -> std::result::Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client.post(&format!("{}/api/request", server))
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
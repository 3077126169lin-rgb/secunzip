//! SecUnzip 客户端：应用状态、业务逻辑与渲染入口。
use crate::api;
use crate::model::{Action, AppConfig, AppState, Mode, OpenTab, PackedEntry, RememberedPassword};
use crate::monitor::{local_ip, Monitor};
use crate::theme;
use eframe::egui;
use secunzip::runtime::{MountHandle, VirtualFS};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// 内容列最大宽度：三个页面共用，保证页面之间左右边距一致
const CONTENT_MAX_W: f32 = 760.0;

pub struct SecUnzipApp {
    pub(crate) state: AppState,
    pub(crate) mode: Mode,
    pub(crate) open_tab: OpenTab,

    pub(crate) user_id: String,
    pub(crate) server_url: String,

    pub(crate) current_file: Option<PathBuf>,
    pub(crate) blackbox: Option<Vec<u8>>,
    pub(crate) app_id: String,
    pub(crate) secret: String,
    pub(crate) is_admin: bool,

    pub(crate) request_days: String,
    pub(crate) request_message: String,
    pub(crate) show_request_opts: bool,

    /// 取密钥口令（普通用户；管理员凭 secret 不需要）
    pub(crate) open_password: String,
    /// 服务端因口令拒绝后才显示口令输入框，避免对无需口令的文件强加输入
    pub(crate) show_password_input: bool,
    /// 上一次口令被拒的原因（原样取自服务端 message）
    pub(crate) password_hint: String,
    /// 「记住口令」勾选框；默认不勾，只有取密钥成功后才写入配置
    pub(crate) remember_password: bool,
    /// 本次自动填入的口令来自哪条记忆（app_id, server_url, user_id），失效时据此精确删除
    pub(crate) remembered_source: Option<(String, String, String)>,
    /// 本机记住的口令（config.json 里明文保存）
    pub(crate) remembered: Vec<RememberedPassword>,
    /// 记住的管理口令（与命令行共用同一份 config.json 的顶层 admin_password 字段），
    /// 只作保存时原样写回，不能在 GUI 保存时被抹掉
    pub(crate) admin_password: Option<String>,
    /// 管理页只在首次进入时用记住的管理口令预填一次，之后用户清空不会被反复填回
    pub(crate) admin_prefill_done: bool,
    /// 配置文件里其它（命令行写入的）顶层字段，保存时原样带回，避免跨端丢字段
    pub(crate) extra: serde_json::Map<String, serde_json::Value>,

    pub(crate) grant_user: String,
    pub(crate) grant_expires: String,
    /// 授权时为对方设定的口令
    pub(crate) grant_password: String,
    /// 审批时可选的口令；留空表示沿用申请人自己设定的口令
    pub(crate) approve_password: String,

    pub(crate) status_message: String,
    pub(crate) status_is_error: bool,

    pub(crate) pending_requests: Vec<api::RequestInfo>,
    pub(crate) monitor: Arc<Monitor>,
    pub(crate) packed_list: Vec<PackedEntry>,

    pub(crate) vfs: Option<Arc<VirtualFS>>,
    pub(crate) mount_handle: Option<MountHandle>,
    pub(crate) cur_dir: String,
    pub(crate) preview_path: Option<String>,
    pub(crate) preview_text: String,
    pub(crate) pending_action: Option<Action>,

    pub(crate) pack_sources: Vec<PathBuf>,
    pub(crate) pack_output: PathBuf,
    pub(crate) pack_allow_temp: bool,
    pub(crate) pack_blackbox: bool,
    pub(crate) pack_key: String,
    pub(crate) pack_flow_machine: bool,
    pub(crate) pack_flow_user: bool,
    pub(crate) pack_flow_date: bool,
    pub(crate) pack_flow_hash: usize,
    pub(crate) pack_flow_b64: bool,
}

impl SecUnzipApp {
    pub(crate) fn new(initial_file: Option<PathBuf>) -> Self {
        let (state, user_id, server_url, remembered, admin_password, extra) =
            match AppConfig::load() {
                Some(c) => (
                    AppState::Main,
                    c.user_id,
                    c.server_url,
                    c.remembered_passwords,
                    c.admin_password,
                    c.extra,
                ),
                None => (
                    AppState::Setup,
                    String::new(),
                    String::new(),
                    Vec::new(),
                    None,
                    serde_json::Map::new(),
                ),
            };
        let state = if initial_file.is_some() {
            AppState::Main
        } else {
            state
        };

        let mut app = Self {
            state,
            mode: Mode::Open,
            open_tab: OpenTab::Browse,
            user_id,
            server_url,
            current_file: initial_file,
            blackbox: None,
            app_id: String::new(),
            secret: String::new(),
            is_admin: false,
            request_days: "3".into(),
            request_message: String::new(),
            show_request_opts: false,
            open_password: String::new(),
            show_password_input: false,
            password_hint: String::new(),
            remember_password: false,
            remembered_source: None,
            remembered,
            admin_password,
            admin_prefill_done: false,
            extra,
            grant_user: String::new(),
            grant_expires: String::new(),
            grant_password: String::new(),
            approve_password: String::new(),
            status_message: String::new(),
            status_is_error: false,
            pending_requests: Vec::new(),
            monitor: Monitor::spawn(),
            packed_list: PackedEntry::load_all(),
            vfs: None,
            mount_handle: None,
            cur_dir: String::new(),
            preview_path: None,
            preview_text: String::new(),
            pending_action: None,
            pack_sources: Vec::new(),
            pack_output: PathBuf::new(),
            pack_allow_temp: true,
            pack_blackbox: false,
            pack_key: String::new(),
            pack_flow_machine: false,
            pack_flow_user: false,
            pack_flow_date: false,
            pack_flow_hash: 0,
            pack_flow_b64: false,
        };
        // 黑盒自解压 EXE：runner 自查尾部标记，内存载入内嵌 .secunzip（不落盘）
        if let Ok(exe) = std::env::current_exe() {
            if let Ok(bytes) = std::fs::read(&exe) {
                if let Ok(embedded) = secunzip::runtime::embedded_secunzip(&bytes) {
                    app.blackbox = Some(embedded.to_vec());
                    app.state = AppState::Main;
                }
            }
        }
        if app.current_file.is_some() || app.blackbox.is_some() {
            app.load_file_info();
        }
        // 启动时若无加载文件但有打包记录，默认显示「我的文件」
        if app.current_file.is_none() && app.blackbox.is_none() && !app.packed_list.is_empty() {
            app.open_tab = OpenTab::Mine;
        }
        *app.monitor.url.lock().unwrap() = app.server_url.clone();
        app
    }

    pub(crate) fn show_status(&mut self, msg: &str, is_error: bool) {
        self.status_message = msg.to_string();
        self.status_is_error = is_error;
    }

    /// 清空口令相关输入。
    ///
    /// 口令按「文件 + 用户」授权保存，换文件后旧口令不再适用，
    /// 因此切换文件时一并清掉，避免拿着上一个文件的口令去重试。
    pub(crate) fn reset_password_inputs(&mut self) {
        self.open_password.clear();
        self.show_password_input = false;
        self.password_hint.clear();
        self.remember_password = false;
        self.remembered_source = None;
        self.admin_prefill_done = false;
        self.grant_password.clear();
        self.approve_password.clear();
    }

    /// 检查服务器连通性（阻塞式，仅在关键动作时调用）
    pub(crate) fn server_reachable(&self) -> bool {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(api::ping_server(&self.server_url))
    }

    /// 确保已设置个人 ID；未设置则跳转设置页并提示（避免用户卡住）
    pub(crate) fn ensure_user(&mut self) -> bool {
        if self.user_id.trim().is_empty() {
            self.mode = Mode::Settings;
            self.show_status("请先填写个人 ID（已为你跳到设置页）", true);
            return false;
        }
        true
    }

    pub(crate) fn build_flow_node(
        pass: &str,
        machine: bool,
        user: bool,
        date: bool,
        hash: usize,
        b64: bool,
    ) -> secunzip::core::KeyNode {
        use secunzip::core::{HashAlgo, KeyNode, KeySource, KeyTransform};
        let mut inputs: Vec<KeyNode> = Vec::new();
        if !pass.trim().is_empty() {
            inputs.push(KeyNode::Input(KeySource::Literal(pass.to_string())));
        }
        if machine {
            inputs.push(KeyNode::Input(KeySource::MachineGuid));
        }
        if user {
            inputs.push(KeyNode::Input(KeySource::DomainUser));
        }
        if date {
            inputs.push(KeyNode::Input(KeySource::CurrentDate));
        }
        if inputs.is_empty() {
            inputs.push(KeyNode::Input(KeySource::Literal(String::new())));
        }
        let mut node = if inputs.len() == 1 {
            inputs.pop().unwrap()
        } else {
            KeyNode::Concat(inputs)
        };
        if hash != 3 {
            let algo = match hash {
                1 => HashAlgo::Sha512,
                2 => HashAlgo::Blake3,
                _ => HashAlgo::Sha256,
            };
            node = KeyNode::Transform(KeyTransform::Hash(algo), Box::new(node));
        }
        if b64 {
            node = KeyNode::Transform(KeyTransform::Base64Encode, Box::new(node));
        }
        node
    }

    pub(crate) fn save_settings(&self) {
        AppConfig {
            user_id: self.user_id.clone(),
            server_url: self.server_url.clone(),
            remembered_passwords: self.remembered.clone(),
            // 原样写回：命令行写入的 admin_password 不能因为 GUI 保存而丢失
            admin_password: self.admin_password.clone(),
            // 其它未知顶层字段同样原样带回
            extra: self.extra.clone(),
        }
        .save();
    }

    /// 记录管理口令（授权/审批成功后调用），与命令行共用 config.json 的 admin_password 字段
    pub(crate) fn remember_admin_password(&mut self, password: &str) {
        if password.is_empty() {
            return;
        }
        self.admin_password = Some(password.to_string());
        self.save_settings();
    }

    /// 记下当前文件的口令（只在勾选「记住口令」且取密钥成功后调用）。
    ///
    /// 明文写入 config.json：该文件本来就明文存个人 ID 与服务器地址，这里不做加密。
    pub(crate) fn remember_current_password(&mut self) {
        if self.open_password.is_empty() {
            return;
        }
        let app_id = self.app_id.clone();
        let server_url = self.server_url.clone();
        let user_id = self.user_id.clone();
        // 同一文件 + 同一用户只保留最新一条
        self.remembered
            .retain(|e| !(e.app_id == app_id && e.user_id == user_id));
        self.remembered.insert(
            0,
            RememberedPassword {
                app_id: app_id.clone(),
                server_url: server_url.clone(),
                user_id: user_id.clone(),
                password: self.open_password.clone(),
            },
        );
        // 上限：只保留最近 32 条，避免配置文件无限膨胀
        self.remembered.truncate(32);
        self.remembered_source = Some((app_id, server_url, user_id));
        self.save_settings();
    }

    /// 口令被服务端判为错误时，删掉与之对应的记忆（可能来自「服务器 + 用户」回退条目）。
    ///
    /// 只删「口令值与被拒口令相同」的那条：用户手输了别的口令被拒时，
    /// 不会连带删掉仍然正确的记忆。返回是否真的删除了内容。
    pub(crate) fn forget_rejected_password(&mut self) -> bool {
        let rejected = self.open_password.clone();
        let app_id = self.app_id.clone();
        let user_id = self.user_id.clone();
        let source = self.remembered_source.clone();
        let before = self.remembered.len();
        self.remembered.retain(|e| {
            let from_source = source
                .as_ref()
                .is_some_and(|(a, s, u)| *a == e.app_id && *s == e.server_url && *u == e.user_id);
            let same_file = e.app_id == app_id && e.user_id == user_id;
            !((from_source || same_file) && e.password == rejected)
        });
        let removed = self.remembered.len() != before;
        if removed {
            self.remembered_source = None;
            self.save_settings();
        }
        removed
    }

    /// 载入文件后：本地若记住过该文件（或该服务器 + 该用户）的口令就自动填入，
    /// 用户不必再输。口令是否仍有效由服务端校验，被拒时会清掉（见 views/open.rs）。
    pub(crate) fn apply_remembered_password(&mut self) {
        let Some(entry) = RememberedPassword::find(
            &self.remembered,
            &self.app_id,
            &self.server_url,
            &self.user_id,
        ) else {
            return;
        };
        self.open_password = entry.password.clone();
        self.remembered_source = Some((
            entry.app_id.clone(),
            entry.server_url.clone(),
            entry.user_id.clone(),
        ));
        // 勾选框如实反映本地已记住的状态
        self.remember_password = true;
        // 展示出来，用户能看见、能改、能取消勾选
        self.show_password_input = true;
    }

    pub(crate) fn open_loader(&self) -> Option<secunzip::runtime::RuntimeLoader> {
        if let Some(bytes) = &self.blackbox {
            secunzip::runtime::RuntimeLoader::from_bytes(bytes).ok()
        } else {
            self.current_file
                .as_ref()
                .and_then(|f| secunzip::runtime::RuntimeLoader::from_file(f).ok())
        }
    }

    pub(crate) fn load_file_info(&mut self) {
        // 换文件：清掉上一个文件的口令输入
        self.reset_password_inputs();
        if let Some(bytes) = self.blackbox.clone() {
            if let Ok(loader) = secunzip::runtime::RuntimeLoader::from_bytes(&bytes) {
                if let secunzip::core::AuthMode::Remote(server) = &loader.header().config.auth_mode
                {
                    self.server_url = server.clone();
                }
                if let Ok(exe) = std::env::current_exe() {
                    if let Ok(md5) = secunzip::crypto::file_md5_hex(&exe) {
                        self.app_id = md5;
                    }
                }
                // 之前记住过口令就自动填入
                self.apply_remembered_password();
                self.show_status("黑盒已加载（联网取密钥后浏览）", false);
            } else {
                self.show_status("黑盒数据解析失败", true);
            }
            return;
        }
        let Some(file) = self.current_file.clone() else {
            return;
        };
        match secunzip::runtime::RuntimeLoader::from_file(&file) {
            Ok(loader) => {
                let header = loader.header();
                if let Ok(md5) = secunzip::crypto::file_md5_hex(&file) {
                    self.app_id = md5;
                }
                if let secunzip::core::AuthMode::Remote(server) = &header.config.auth_mode {
                    self.server_url = server.clone();
                }
                let secret_path = file.with_extension("secret");
                if let Ok(s) = std::fs::read_to_string(&secret_path) {
                    let s = s.trim().to_string();
                    if !s.is_empty() {
                        self.secret = s;
                        self.is_admin = true;
                    }
                }
                if self.is_admin {
                    // 管理员凭 secret 取密钥，不需要口令，也不要用记住的口令干扰
                    self.open_password.clear();
                    self.show_password_input = false;
                    self.remember_password = false;
                    self.remembered_source = None;
                } else {
                    // 之前记住过口令就自动填入
                    self.apply_remembered_password();
                }
                self.show_status(
                    if self.is_admin {
                        "文件已加载（你是此文件的管理员）"
                    } else {
                        "文件已加载"
                    },
                    false,
                );
            }
            Err(e) => self.show_status(&format!("读取文件失败: {}", e), true),
        }
    }

    pub(crate) fn refresh_requests(&mut self) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            api::list_requests(&self.server_url, &self.app_id, &self.secret).await
        });
        match result {
            Ok(requests) => {
                *self.monitor.requests.lock().unwrap() = requests.clone();
                self.pending_requests = requests;
                self.show_status("已刷新", false);
            }
            Err(e) => self.show_status(&e.to_string(), true),
        }
    }

    pub(crate) fn start_server(&self) -> (String, bool) {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(d) = &exe_dir {
            candidates.push(d.join("secunzip-server.exe"));
            candidates.push(d.join("deploy").join("server").join("secunzip-server.exe"));
            candidates.push(
                d.join("..")
                    .join("server")
                    .join("target")
                    .join("release")
                    .join("secunzip-server.exe"),
            );
            candidates.push(
                d.join("..")
                    .join("server")
                    .join("target")
                    .join("debug")
                    .join("secunzip-server.exe"),
            );
        }
        candidates.push(PathBuf::from(
            "server\\target\\release\\secunzip-server.exe",
        ));
        candidates.push(PathBuf::from("deploy\\server\\secunzip-server.exe"));
        let Some(exe) = candidates.into_iter().find(|p| p.exists()) else {
            return (
                "未找到 secunzip-server.exe（先运行 deploy\\install-windows.ps1 部署）".into(),
                true,
            );
        };
        match std::process::Command::new(&exe)
            .env("SECUNZIP_PORT", "8090")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => {
                let share = match local_ip() {
                    Some(ip) => format!("接收方用 http://{}:8090 连接（请在下方连接向导填它，并放行防火墙 8090 端口）", ip),
                    None => "如需跨机，请在下方连接向导填可访问地址".to_string(),
                };
                (
                    format!("服务端已在后台启动（绑定 0.0.0.0:8090）· {}", share),
                    false,
                )
            }
            Err(e) => (format!("启动服务端失败: {}", e), true),
        }
    }

    pub(crate) fn do_pack(&mut self) {
        if self.pack_sources.is_empty() {
            self.show_status("请先添加源文件/文件夹", true);
            return;
        }
        if self.pack_output.as_os_str().is_empty() {
            self.show_status("请选择输出文件位置", true);
            return;
        }
        if self.server_url.trim().is_empty() {
            self.mode = Mode::Settings;
            self.show_status("请先填写服务器地址（已为你跳到设置页）", true);
            return;
        }
        // 服务器连通性预检：离线时明确引导，避免打包后才注册失败
        if !self.server_reachable() {
            self.mode = Mode::Settings;
            self.show_status(
                &format!(
                    "服务器未连接（{}）：请在设置页「启动服务端」或核对地址",
                    self.server_url
                ),
                true,
            );
            return;
        }
        self.show_status("正在打包（压缩 + 加密 + 注册服务端）...", false);
        let has_input = !self.pack_key.trim().is_empty()
            || self.pack_flow_machine
            || self.pack_flow_user
            || self.pack_flow_date;
        let custom_key = if !has_input {
            None
        } else {
            let node = Self::build_flow_node(
                &self.pack_key,
                self.pack_flow_machine,
                self.pack_flow_user,
                self.pack_flow_date,
                self.pack_flow_hash,
                self.pack_flow_b64,
            );
            secunzip::key_derive::generate_key_from_flow(&node)
                .ok()
                .filter(|k| !k.is_empty())
        };
        let result = secunzip::cli::commands::pack_file(
            self.pack_sources.clone(),
            self.pack_output.clone(),
            self.server_url.clone(),
            self.pack_allow_temp,
            custom_key,
            self.pack_blackbox,
        );
        match result {
            Ok(out) => {
                let short = &out.app_id[..8.min(out.app_id.len())];
                let secret_path = self.pack_output.with_extension("secret");
                if out.registered {
                    self.show_status(
                        &format!("已打包并注册（文件ID {}）· 把文件发给接收方，对方用个人 ID 申请后即可打开 · 管理密钥已存 {}（妥善保管，用于授权管理）", short, secret_path.display()),
                        false,
                    );
                } else {
                    self.show_status(
                        &format!(
                            "打包完成但注册服务端失败（暂无法联网打开）· 文件ID {}",
                            short
                        ),
                        true,
                    );
                }
                // 记录到「我打包的文件」清单
                let size = std::fs::metadata(&self.pack_output)
                    .map(|m| m.len())
                    .unwrap_or(0);
                let entry = PackedEntry {
                    name: self
                        .pack_output
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    path: self.pack_output.clone(),
                    app_id: out.app_id.clone(),
                    secret: secret_path.clone(),
                    size,
                    created_at: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
                };
                PackedEntry::add(entry);
                self.packed_list = PackedEntry::load_all();
                self.current_file = Some(self.pack_output.clone());
                self.load_file_info();
                self.mode = Mode::Open;
            }
            Err(e) => self.show_status(&format!("打包失败: {}", e), true),
        }
    }

    pub(crate) fn apply_pending_action(&mut self) {
        match self.pending_action.take() {
            Some(Action::EnterDir(path)) => {
                self.cur_dir = path;
                self.preview_path = None;
            }
            Some(Action::Preview(path)) => {
                if let Some(vfs) = &self.vfs {
                    if let Ok(data) = vfs.read_file(&path) {
                        self.preview_text = if VirtualFS::is_text_file(&path) {
                            String::from_utf8_lossy(data).into_owned()
                        } else {
                            String::new()
                        };
                    }
                }
                self.preview_path = Some(path);
            }
            None => {}
        }
    }
}

// ============================================================================
// 渲染
// ============================================================================

impl eframe::App for SecUnzipApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        theme::apply(ctx);
        // 后台监控同步：URL 与待审批自动拉取开关；周期重绘以刷新状态点
        *self.monitor.url.lock().unwrap() = self.server_url.clone();
        *self.monitor.app_id.lock().unwrap() = self.app_id.clone();
        *self.monitor.secret.lock().unwrap() = self.secret.clone();
        let manage_active = self.state == AppState::Main
            && self.mode == Mode::Open
            && self.open_tab == OpenTab::Manage
            && self.is_admin;
        self.monitor
            .auto_requests
            .store(manage_active, Ordering::Relaxed);
        if manage_active {
            if let Ok(list) = self.monitor.requests.lock() {
                self.pending_requests = list.clone();
            }
        }
        ctx.request_repaint_after(Duration::from_secs(2));

        if self.state == AppState::Setup {
            self.show_setup(ctx);
            self.show_status_bar(ctx);
            return;
        }

        self.show_top_bar(ctx);
        self.show_status_bar(ctx);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(20.0, 10.0)),
            )
            .show(ctx, |ui| {
                ui.add_space(6.0);
                theme::centered(ui, CONTENT_MAX_W, |ui| match self.mode {
                    Mode::Open => self.show_open(ui),
                    Mode::Pack => self.show_pack(ui),
                    Mode::Settings => self.show_settings(ui),
                });
            });

        self.apply_pending_action();
    }
}

/// 服务端因口令问题拒绝时（缺失 / 错误 / 该授权记录本就没有口令），
/// message 里都会点明「口令」，据此决定是否展示口令输入框。
pub(crate) fn is_password_refusal(msg: &str) -> bool {
    msg.contains("口令") || msg.contains("密码") || msg.to_ascii_lowercase().contains("password")
}

/// 服务端明确判「口令不对」，区别于「没收到口令」和「该授权根本没有口令」。
/// 只有这一种情况才需要清掉本机记住的口令。
pub(crate) fn is_wrong_password(msg: &str) -> bool {
    msg.contains("口令错误")
        || msg.contains("口令不正确")
        || msg.contains("密码错误")
        || msg.to_ascii_lowercase().contains("wrong password")
}

pub(crate) fn short_id(id: &str) -> String {
    if id.is_empty() {
        "—".into()
    } else {
        id[..8.min(id.len())].to_string()
    }
}

pub(crate) fn human_size(size: usize) -> String {
    if size < 1024 {
        format!("{} B", size)
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    }
}

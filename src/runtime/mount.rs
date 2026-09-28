//! 内存只读虚拟盘挂载（WebDAV）
//!
//! 把解密后的 VirtualFS 以「只读 WebDAV」在本机回环地址服务出来，
//! 再用 Windows 自带的 WebClient（`net use`）映射成资源管理器里的驱动器。
//! 全程内存、只读、不落盘，卸载即释放，防止复制扩散。
//!
//! 为什么不用真正的 ISO 挂载：
//! 一是 Windows 的 `Mount-DiskImage` 只能挂载磁盘上已存在的镜像文件，要用它就得先把解密后的
//! 明文写到磁盘，与「明文不落盘」直接冲突；二是真正的内存盘挂载需要 WinFsp / Dokan 之类的
//! 文件系统驱动或签名内核驱动，超出本项目「零第三方运行时」的依赖范围。
//! 回环 WebDAV 只需系统自带的 WebClient 组件，因此成为实际采用的方式。
//!
//! 盘符不写死：`pick_free_drive` 用 `GetLogicalDrives` 枚举系统实际占用的盘符，
//! 从 `Z:` 向下取第一个空闲者（排除 A:/B: 与系统盘），避免与用户已有的
//! 网络共享、U 盘或 VHD 撞车。
//!
//! 说明：Windows 自带的 WebDAV 迷你重定向器对匿名/HTTP 偶有挑剔，
//! 故 `net use` 失败时 WebDAV 服务仍会继续运行，调用方可用返回的 URL
//! 手动「映射网络驱动器」。但失败不会被当成成功：返回类型为
//! `Result<MountHandle, MountError>`：
//! - `Ok(handle)`：服务在跑且本次映射成功，`handle.is_mapped()` 为 true；
//! - `Err(MountError::MapFailed { handle, message })`：服务仍在跑，但映射失败
//!   （或本机无空闲盘符）。`message` 是含 WebDAV URL 的中文提示，调用方应保留
//!   `handle`（用于停止服务）并提示用户手动映射；
//! - `Err(MountError::Service(..))`：服务没起来，没有句柄。
//!
//! 安全约束：`MountHandle` 记录映射是否由本程序建立，`unmount` 只在
//! `is_mapped()` 为 true 时才执行 `net use <盘符> /delete /y`，
//! 绝不会删除用户自己已有的盘符映射。
//!
//! # 访问控制（为什么 URL 里有 token、为什么要校验 Host）
//!
//! 这个服务没有账号体系（Windows WebClient 走匿名/HTTP 最省事），因此改为
//! 两道「能力证明」：
//!
//! 1. **挂载期随机 token 前缀**：每次 `start_webdav` 都用操作系统 CSPRNG
//!    （`rand::rngs::OsRng`）生成 16 字节随机数并 hex 编码成 32 个字符，
//!    服务地址是 `http://127.0.0.1:<端口>/<token>/`，`MountHandle::url()` 返回带
//!    token 的地址。凡是路径不以该 token 段开头的请求一律 403。
//!    这样「端口被扫到」不再等于「明文被读到」——本机其它进程必须先拿到 token。
//! 2. **Host 头校验**：只接受 `127.0.0.1:<本服务端口>` 与 `localhost:<本服务端口>`，
//!    其余（含任意域名、其它端口的回环）一律 403。这条是**专门用来掐断 DNS
//!    rebinding 的**：恶意网页把 attack.com 解析到 127.0.0.1 后，浏览器发出的请求
//!    里 Host 仍是 `attack.com`，因此拿不到内容；而浏览器无法把 Host 伪造成
//!    `127.0.0.1:<端口>` 再去读响应。
//!
//! token 比较采用逐字节异或累加（近似常数时间），避免按前缀逐位比较被计时侧信道
//! 逐字符还原。请求解析另加：Content-Length 上限、socket 读超时（防 slowloris）、
//! 显式拒绝 `..` 段与 NUL 字节。
//!
//! # 仍然防不住什么（如实说明）
//!
//! 本机制关闭的是「浏览器 / DNS rebinding 远程读取」与「随机端口扫描式顺手牵羊」
//! 两条路，并**不是**一个完整的访问控制：
//! - token 虽然是 CSPRNG 生成、不可预测，但它会**以明文形式出现在若干本机可见的
//!   地方**：GUI 状态栏文字、`net use` 命令行（因而出现在进程列表、命令历史与
//!   WebClient 日志里）、`MountHandle::url()` 的返回值。任何读到这些的本机进程
//!   （或用户自己）都能完整读取明文。**改用 OsRng 不会消除这条**——token 一旦
//!   被打印/传参就与生成方式无关了；
//! - Windows 盘符映射一旦建立，资源管理器/任何能读该驱动器的进程都能读到内容，
//!   直到 `unmount` 为止；这是「映射成盘」这一功能的固有代价；
//! - 服务是明文 HTTP（回环），本机抓包可以直接拿到 token 与内容；
//! - 能读取本进程内存（同用户调试器、注入、内存转储）的攻击者可以直接拿走明文
//!   与 token，不在本模块的防御范围内。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use super::vfs::VirtualFS;
use crate::Result;

/// 挂载句柄：卸载时停止服务并断开驱动器
pub struct MountHandle {
    stop: Arc<AtomicBool>,
    port: u16,
    /// 本次挂载的访问 token（URL 前缀，见模块文档）
    token: String,
    drive: String,
    /// 本次映射是否由本程序建立；卸载时据此决定是否删除盘符
    mapped: bool,
    server: Option<JoinHandle<()>>,
}

impl MountHandle {
    pub fn port(&self) -> u16 {
        self.port
    }
    /// 请求映射的盘符（形如 `Z:`）；无空闲盘符时为空串
    pub fn drive(&self) -> &str {
        &self.drive
    }
    /// 本次挂载是否真的建立了驱动器映射
    pub fn is_mapped(&self) -> bool {
        self.mapped
    }
    /// 本机 WebDAV 地址（可手动「映射网络驱动器」）
    ///
    /// 形如 `http://127.0.0.1:<端口>/<token>/`：token 必须留在 URL 里，
    /// 否则服务会以 403 拒绝。
    pub fn url(&self) -> String {
        base_url(self.port, &self.token)
    }
}

/// 服务的根地址（含 token 前缀）
fn base_url(port: u16, token: &str) -> String {
    format!("http://127.0.0.1:{}/{}/", port, token)
}

/// 挂载失败：区分「服务没起来」与「服务在跑但映射失败」
pub enum MountError {
    /// WebDAV 服务无法启动，未建立任何映射
    Service(String),
    /// WebDAV 服务在运行，但驱动器映射失败（含无空闲盘符）；
    /// `handle` 仍需交回调用方，用于停止服务与展示 URL
    MapFailed {
        handle: MountHandle,
        message: String,
    },
}

impl MountError {
    /// 失败时仍存活的服务句柄（仅映射失败时存在）
    pub fn into_handle(self) -> Option<MountHandle> {
        match self {
            MountError::MapFailed { handle, .. } => Some(handle),
            MountError::Service(_) => None,
        }
    }
}

impl std::fmt::Display for MountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MountError::Service(m) => write!(f, "WebDAV 服务启动失败: {}", m),
            MountError::MapFailed { message, .. } => write!(f, "{}", message),
        }
    }
}

impl std::fmt::Debug for MountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MountError::Service(m) => write!(f, "MountError::Service({})", m),
            MountError::MapFailed { message, .. } => {
                write!(f, "MountError::MapFailed({})", message)
            }
        }
    }
}

impl std::error::Error for MountError {}

impl Drop for MountHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.server.take() {
            let _ = t.join();
        }
    }
}

/// 读超时：防止 slowloris 客户端慢慢发请求、长期霸占一个线程。
/// 本地回环正常请求在毫秒级完成，10 秒足够宽松，又不会让线程永久卡住。
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Content-Length 上限：本服务只读，不接受上传，超过 4 KiB 一律 413。
const MAX_CONTENT_LENGTH: usize = 4 * 1024;

/// 生成一次挂载用的随机 token（16 字节 → 32 个十六进制字符，128 位）。
///
/// token 是挂载期间**唯一的访问控制凭据**：只要它可预测，本机任何进程都能算出
/// 它并读走全部明文。因此这里用操作系统的 CSPRNG（`rand::rngs::OsRng`，直接读
/// 系统熵源），而不是时间/进程号/地址之类的「混合哈希」——那种做法只是把可猜测
/// 的输入搅乱，输出仍不具备密码学不可预测性。
fn random_token() -> String {
    use rand::rngs::OsRng;
    use rand::RngCore;
    use std::fmt::Write as _;

    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    let mut token = String::with_capacity(32);
    for b in bytes {
        // 写入 String 不会失败（fmt::Write for String 不会返回 Err）
        let _ = write!(token, "{:02x}", b);
    }
    token
}

/// 启动只读 WebDAV 服务（后台线程，随机回环端口）
///
/// 返回 `(端口, token, 停止位, 线程句柄)`；token 是本次挂载的 URL 前缀。
fn start_webdav(vfs: Arc<VirtualFS>) -> Result<(u16, String, Arc<AtomicBool>, JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let token = random_token();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let token2 = token.clone();
    let handle = std::thread::spawn(move || loop {
        if stop2.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let vfs = vfs.clone();
                let token = token2.clone();
                std::thread::spawn(move || {
                    let _ = handle_conn(stream, &vfs, &token, port);
                });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    });
    Ok((port, token, stop, handle))
}

// ===== 盘符选择 =====

#[cfg(windows)]
extern "system" {
    /// kernel32：返回盘符占用位图，bit0=A: … bit25=Z:
    fn GetLogicalDrives() -> u32;
}

/// 从位图里挑第一个空闲盘符：从 Z: 向下找，排除 A:/B: 与系统盘
fn pick_free_from_mask(mask: u32, system: Option<u8>) -> Option<String> {
    for letter in (2u8..=25).rev() {
        if Some(letter) == system {
            continue;
        }
        if mask & (1u32 << letter) == 0 {
            return Some(format!("{}:", (b'A' + letter) as char));
        }
    }
    None
}

/// 系统盘盘符（0=A … 25=Z），取自 `SystemDrive` 环境变量
fn system_drive_index() -> Option<u8> {
    let v = std::env::var("SystemDrive").ok()?;
    let b = v.trim().as_bytes().first().copied()?;
    let up = b.to_ascii_uppercase();
    if up.is_ascii_uppercase() {
        Some(up - b'A')
    } else {
        None
    }
}

/// 选一个本机空闲盘符（形如 `Z:`），无可用盘符返回 None。
///
/// 依据 `GetLogicalDrives` 报告的占用位图，而非 `Path::exists()`：
/// 已断开但仍占着盘符的网络映射同样会出现在位图里，用文件系统探测会漏判。
/// 排除 A:/B:（软驱）与系统盘，从 `Z:` 向下找。
#[cfg(windows)]
pub fn pick_free_drive() -> Option<String> {
    let mask = unsafe { GetLogicalDrives() };
    pick_free_from_mask(mask, system_drive_index())
}

/// 非 Windows 没有盘符概念
#[cfg(not(windows))]
pub fn pick_free_drive() -> Option<String> {
    None
}

/// 形如 `Z:` 的单字母盘符
fn is_drive_letter(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// 执行 `net use` 映射；失败返回中文原因（含 URL）
#[cfg(windows)]
fn map_drive(drive: &str, url: &str) -> std::result::Result<(), String> {
    let out = std::process::Command::new("net")
        .args(["use", drive, url, "/user:guest", "guest", "/persistent:no"])
        .output()
        .map_err(|e| format!("无法执行 net 命令（{} -> {}）: {}", drive, url, e))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let out_text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let detail = if err.is_empty() { out_text } else { err };
    Err(format!("映射驱动器 {} 失败（{}）: {}", drive, url, detail))
}

/// 把 VirtualFS 挂载为资源管理器里的驱动器（内存只读）
///
/// `drive` 为 `pick_free_drive()` 得到的盘符；传 None 表示本机无空闲盘符，
/// 此时只启动 WebDAV 服务，并通过 `MountError::MapFailed` 返回 URL 与句柄。
///
/// 失败语义见模块文档：`net use` 失败不再当作成功，但服务继续运行，
/// 句柄随错误一并返回，调用方须保留它以便停止服务。
#[cfg(windows)]
pub fn mount_vfs_to_drive(
    vfs: Arc<VirtualFS>,
    drive: Option<&str>,
) -> std::result::Result<MountHandle, MountError> {
    let (port, token, stop, server) =
        start_webdav(vfs).map_err(|e| MountError::Service(e.to_string()))?;
    let url = base_url(port, &token);
    let mut handle = MountHandle {
        stop,
        port,
        token,
        drive: drive.unwrap_or("").to_string(),
        mapped: false,
        server: Some(server),
    };
    let result = match drive {
        Some(d) if is_drive_letter(d) => {
            // 用 Windows 自带 WebClient 映射为驱动器；/user 携带凭据以避免交互式提示。
            map_drive(d, &url)
        }
        _ => Err("本机没有可用的空闲盘符".to_string()),
    };
    match result {
        Ok(()) => {
            handle.mapped = true;
            Ok(handle)
        }
        Err(why) => Err(MountError::MapFailed {
            handle,
            message: format!(
                "{}。WebDAV 服务仍在运行，可手动「映射网络驱动器」填 {}",
                why, url
            ),
        }),
    }
}

/// 非 Windows：仅启动 WebDAV 服务（无驱动器映射）
#[cfg(not(windows))]
pub fn mount_vfs_to_drive(
    vfs: Arc<VirtualFS>,
    drive: Option<&str>,
) -> std::result::Result<MountHandle, MountError> {
    let (port, token, stop, server) =
        start_webdav(vfs).map_err(|e| MountError::Service(e.to_string()))?;
    Ok(MountHandle {
        stop,
        port,
        token,
        drive: drive.unwrap_or("").to_string(),
        mapped: false,
        server: Some(server),
    })
}

/// 卸载：断开本程序建立的映射 + 停止服务
///
/// 只有 `mapped` 为 true 才会执行 `net use <盘符> /delete /y`：
/// 映射不是本程序建的（失败、无盘符、非 Windows）时绝不碰盘符，
/// 避免删掉用户自己已有的网络共享或 U 盘映射。
pub fn unmount(h: MountHandle) {
    #[cfg(windows)]
    {
        if h.mapped && is_drive_letter(&h.drive) {
            let _ = std::process::Command::new("net")
                .args(["use", &h.drive, "/delete", "/y"])
                .output();
        }
    }
    // 置停止位；Drop 里 join 线程
    drop(h);
}

// ===== WebDAV / HTTP 处理 =====

/// 处理一条连接。
///
/// 顺序很重要：先设读超时（防 slowloris），再解析请求行与头部，然后**先校验
/// Host、再校验 token**，最后才碰 VFS。任何一步不通过都写一条短响应并断开。
fn handle_conn(stream: TcpStream, vfs: &VirtualFS, token: &str, port: u16) -> std::io::Result<()> {
    // 慢速客户端（slowloris）不能靠一个连接长期占住线程：读写都设上限。
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(READ_TIMEOUT));

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        // 连接被对端直接关闭，无需响应
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let raw_path = parts.next().unwrap_or("/").to_string();

    // 读头部到空行，取 Host 与 Content-Length
    let mut content_length = 0usize;
    let mut host: Option<String> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let t = line.trim_end();
        if t.is_empty() {
            break;
        }
        let lower = t.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = lower.strip_prefix("host:") {
            host = Some(v.trim().to_string());
        }
    }

    let mut out = stream;

    // (1) Host 校验：这是防 DNS rebinding 的关键一步。
    // 只认本机回环名 + 本服务端口，别的（攻击者域名、别的回环端口）一律 403。
    if !host_is_allowed(host.as_deref(), port) {
        return refuse(&mut out, 403, "Forbidden", b"403 forbidden host");
    }

    // (2) Content-Length 上限：只读服务不需要 body，超过 4 KiB 直接 413，
    // 并且不去读 body（避免拿超大 Content-Length 逼服务端分配内存）。
    if content_length > MAX_CONTENT_LENGTH {
        return refuse(&mut out, 413, "Payload Too Large", b"413 payload too large");
    }
    // 丢弃 body（PROPFIND 的 prop 请求体可忽略）
    if content_length > 0 {
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body)?;
    }

    // (3) token 校验 + 路径安全检查（`..` / NUL）。失败一律 403。
    let Some(vpath) = resolve_target(&raw_path, token) else {
        return refuse(&mut out, 403, "Forbidden", b"403 forbidden");
    };

    match method.as_str() {
        "OPTIONS" => {
            write_resp(
                &mut out,
                200,
                "OK",
                &[("DAV", "1,2"), ("Allow", "OPTIONS,GET,HEAD,PROPFIND")],
                0,
                b"",
            )?;
        }
        "PROPFIND" => {
            // 路径不存在时必须是 404，而不是一个空的 207：
            // 否则客户端会把「不存在」当成「空集合」。
            if !vfs_exists(vfs, &vpath) {
                write_resp(&mut out, 404, "Not Found", &[], 0, b"")?;
            } else {
                let xml = multistatus_xml(vfs, &vpath, &format!("/{}", token));
                write_resp(
                    &mut out,
                    207,
                    "Multi-Status",
                    &[("Content-Type", "application/xml; charset=\"utf-8\"")],
                    xml.len(),
                    xml.as_bytes(),
                )?;
            }
        }
        "GET" | "HEAD" => match vfs.read_file(&vpath) {
            Ok(data) => {
                let len = data.len();
                let body: &[u8] = if method == "HEAD" { &[] } else { data };
                write_resp(
                    &mut out,
                    200,
                    "OK",
                    &[("Content-Type", "application/octet-stream")],
                    len,
                    body,
                )?;
            }
            Err(_) => write_resp(&mut out, 404, "Not Found", &[], 0, b"")?,
        },
        _ => write_resp(&mut out, 405, "Method Not Allowed", &[], 0, b"")?,
    }
    out.flush()?;
    let _ = out.shutdown(Shutdown::Both);
    Ok(())
}

/// 写一条短拒绝响应并关闭连接
fn refuse(stream: &mut TcpStream, code: u16, msg: &str, body: &[u8]) -> std::io::Result<()> {
    write_resp(
        stream,
        code,
        msg,
        &[("Content-Type", "text/plain; charset=\"utf-8\"")],
        body.len(),
        body,
    )?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

/// Host 头是否被接受：只允许 `127.0.0.1:<本服务端口>` 与 `localhost:<本服务端口>`
/// （主机名大小写不敏感，端口必须显式且等于本服务端口）。
///
/// 为什么这样就能杀掉 DNS rebinding：浏览器发请求时 Host 由 URL 的主机名决定，
/// 攻击者无法把它改成 `127.0.0.1:<端口>`。被 rebinding 的页面无论把域名解析到
/// 哪里，请求里的 Host 仍是攻击者域名，于是这里直接 403，读不到任何明文。
fn host_is_allowed(host: Option<&str>, port: u16) -> bool {
    let Some(h) = host else { return false };
    let h = h.trim();
    // IPv6 字面量（`[::1]:port`）不支持：服务只绑 IPv4 回环
    let (name, port_part) = match h.rsplit_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (h, None),
    };
    if !(name.eq_ignore_ascii_case("127.0.0.1") || name.eq_ignore_ascii_case("localhost")) {
        return false;
    }
    // 必须带端口，且等于本服务端口（HTTP/1.1 下非默认端口一定出现在 Host 里）
    match port_part {
        Some(p) => p == port.to_string(),
        None => false,
    }
}

/// 近似常数时间的字符串比较：逐字节异或后累加，不提前 return。
///
/// 用于 token 校验，避免按前缀逐字符比较时通过响应耗时还原 token。
/// （长度不同会走 `len` 分支提前返回，只泄露长度——token 长度固定为 32，
/// 不构成有效信息。）
fn ct_eq(a: &str, b: &str) -> bool {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    if x.len() != y.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in x.iter().zip(y.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// 请求路径 → VirtualFS 路径：校验并剥离 token 前缀。
///
/// 返回 `None` 表示应拒绝（403）：
/// - 路径含 NUL 字节（原样或百分号编码后）；
/// - 路径含 `..` 段或 `\..\`（显式拒绝穿越，不依赖 VFS 恰好没有这种键——
///   那是巧合而非设计）；
/// - 第一段与 token 不相等（近似常数时间比较）。
fn resolve_target(raw: &str, token: &str) -> Option<String> {
    if raw.contains('\0') {
        return None;
    }
    // normalize_path 会去查询串、百分号解码、去首尾 `/`
    let p = normalize_path(raw);
    if p.contains('\0') {
        return None;
    }
    // 解码之后再查 `..`，这样 `%2e%2e` 也逃不掉
    if p.split(['/', '\\']).any(|seg| seg == "..") {
        return None;
    }
    let (first, tail) = match p.split_once('/') {
        Some((a, b)) => (a, b),
        None => (p.as_str(), ""),
    };
    if !ct_eq(first, token) {
        return None;
    }
    Some(tail.to_string())
}

/// 路径在 VFS 里是否存在（文件或目录）。
///
/// 根路径恒存在；目录的判定用 `list_dir` 非空——VFS 由文件列表构建，
/// 非根目录至少含一个条目，因此不会把「空目录」误判为不存在。
fn vfs_exists(vfs: &VirtualFS, path: &str) -> bool {
    path.is_empty() || vfs.read_file(path).is_ok() || !vfs.list_dir(path).is_empty()
}

fn write_resp(
    stream: &mut TcpStream,
    code: u16,
    msg: &str,
    headers: &[(&str, &str)],
    content_length: usize,
    body: &[u8],
) -> std::io::Result<()> {
    let mut resp = format!("HTTP/1.1 {} {}\r\n", code, msg);
    for (k, v) in headers {
        resp.push_str(k);
        resp.push_str(": ");
        resp.push_str(v);
        resp.push_str("\r\n");
    }
    resp.push_str("Content-Length: ");
    resp.push_str(&content_length.to_string());
    resp.push_str("\r\nConnection: close\r\n\r\n");
    stream.write_all(resp.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}

/// URL 路径 → VirtualFS 路径（去查询串、百分号解码、去首尾 /）
fn normalize_path(raw: &str) -> String {
    let p = raw.split('?').next().unwrap_or(raw);
    let p = percent_decode(p);
    p.trim_start_matches('/').trim_end_matches('/').to_string()
}

/// 百分号解码。
///
/// 按**字节**处理而非 `&s[i + 1..i + 3]`：后者在多字节 UTF-8 字符上可能切到
/// 非字符边界而 panic（例如 `/%é` 会让服务线程 panic），属于解析器加固的一部分。
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 生成 WebDAV Multi-Status（自身 + 子项）
///
/// `prefix` 是本次挂载的 token 前缀（形如 `/abcd...`）。href 必须带上它，
/// 否则 WebClient 会按相对路径去请求 `/readme.txt` 这类没有 token 的地址并被 403。
fn multistatus_xml(vfs: &VirtualFS, path: &str, prefix: &str) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">\n",
    );

    let self_data = vfs.read_file(path).ok();
    let self_is_file = self_data.is_some();
    let name = path.rsplit('/').next().unwrap_or("").to_string();
    push_response(
        &mut xml,
        &href_for(prefix, path),
        &name,
        self_is_file,
        self_data.map(|d| d.len()).unwrap_or(0),
    );

    if !self_is_file {
        for e in vfs.list_dir(path) {
            push_response(
                &mut xml,
                &href_for(prefix, &e.path),
                &e.name,
                !e.is_dir,
                e.size,
            );
        }
    }

    xml.push_str("</D:multistatus>");
    xml
}

/// VFS 路径 → 带 token 前缀的 href
fn href_for(prefix: &str, path: &str) -> String {
    if path.is_empty() {
        format!("{}/", prefix)
    } else {
        format!("{}/{}", prefix, path)
    }
}

fn push_response(xml: &mut String, href: &str, name: &str, is_file: bool, size: usize) {
    xml.push_str("<D:response>\n");
    xml.push_str(&format!("<D:href>{}</D:href>\n", xml_escape(href)));
    xml.push_str("<D:propstat>\n<D:prop>\n");
    xml.push_str(&format!(
        "<D:displayname>{}</D:displayname>\n",
        xml_escape(name)
    ));
    if is_file {
        xml.push_str("<D:resourcetype/>\n");
        xml.push_str(&format!(
            "<D:getcontentlength>{}</D:getcontentlength>\n",
            size
        ));
    } else {
        xml.push_str("<D:resourcetype><D:collection/></D:resourcetype>\n");
    }
    xml.push_str("</D:prop>\n<D:status>HTTP/1.1 200 OK</D:status>\n</D:propstat>\n</D:response>\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_vfs() -> Arc<VirtualFS> {
        Arc::new(VirtualFS::from_files(vec![
            ("readme.txt".to_string(), b"Hello VFS".to_vec()),
            ("src/main.rs".to_string(), b"fn main() {}".to_vec()),
        ]))
    }

    /// 测试用服务端：起服务并记住端口/token，测试结束调 `shutdown`。
    struct TestServer {
        port: u16,
        token: String,
        stop: Arc<AtomicBool>,
        join: JoinHandle<()>,
    }

    impl TestServer {
        fn start() -> Self {
            let (port, token, stop, join) = start_webdav(sample_vfs()).unwrap();
            Self {
                port,
                token,
                stop,
                join,
            }
        }

        /// 带 token 的合法路径
        fn path(&self, p: &str) -> String {
            format!("/{}/{}", self.token, p.trim_start_matches('/'))
        }

        /// 原样发送一段请求，返回完整响应文本
        fn raw(&self, request: &str) -> String {
            let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
            s.write_all(request.as_bytes()).unwrap();
            let _ = s.shutdown(Shutdown::Write);
            let mut resp = String::new();
            let _ = s.read_to_string(&mut resp);
            resp
        }

        /// 用合法 Host 发请求
        fn call(&self, method: &str, path: &str) -> (u16, String) {
            let host = format!("127.0.0.1:{}", self.port);
            self.call_host(method, path, &host)
        }

        /// 指定 Host 发请求
        fn call_host(&self, method: &str, path: &str, host: &str) -> (u16, String) {
            let req = format!(
                "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                method, path, host
            );
            let resp = self.raw(&req);
            (status_of(&resp), body_of(&resp))
        }

        fn shutdown(self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = self.join.join();
        }
    }

    fn status_of(resp: &str) -> u16 {
        resp.split_whitespace()
            .nth(1)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    fn body_of(resp: &str) -> String {
        resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string()
    }

    #[test]
    fn webdav_get_file() {
        let srv = TestServer::start();
        let (status, body) = srv.call("GET", &srv.path("readme.txt"));
        srv.shutdown();
        assert_eq!(status, 200);
        assert_eq!(body, "Hello VFS");
    }

    #[test]
    fn webdav_get_missing_is_404() {
        let srv = TestServer::start();
        let (status, _) = srv.call("GET", &srv.path("nope.txt"));
        srv.shutdown();
        assert_eq!(status, 404);
    }

    #[test]
    fn webdav_propfind_root_lists_entries() {
        let srv = TestServer::start();
        let (status, body) = srv.call("PROPFIND", &srv.path(""));
        srv.shutdown();
        assert_eq!(status, 207);
        assert!(body.contains("readme.txt"), "根应含 readme.txt: {}", body);
        assert!(body.contains("src"), "根应含 src: {}", body);
        assert!(body.contains("<D:collection/>"), "src 是目录: {}", body);
    }

    /// href 必须带 token 前缀，否则 WebClient 会去请求无 token 的地址
    #[test]
    fn webdav_propfind_hrefs_carry_token() {
        let srv = TestServer::start();
        let (status, body) = srv.call("PROPFIND", &srv.path(""));
        let token = srv.token.clone();
        srv.shutdown();
        assert_eq!(status, 207);
        assert!(
            body.contains(&format!("<D:href>/{}/readme.txt</D:href>", token)),
            "href 应含 token 前缀: {}",
            body
        );
    }

    #[test]
    fn webdav_options_has_dav_header() {
        let srv = TestServer::start();
        let (status, _) = srv.call("OPTIONS", &srv.path(""));
        srv.shutdown();
        assert_eq!(status, 200);
    }

    // ===== 访问控制 =====

    /// 不带 token 的请求必须被拒（旧的 `http://127.0.0.1:port/readme.txt` 现在应当失败）
    #[test]
    fn webdav_rejects_missing_token() {
        let srv = TestServer::start();
        let (status, _) = srv.call("GET", "/readme.txt");
        let (propfind_status, _) = srv.call("PROPFIND", "/");
        let (options_status, _) = srv.call("OPTIONS", "/");
        srv.shutdown();
        assert_eq!(status, 403, "无 token 的 GET 必须 403");
        assert_eq!(propfind_status, 403, "无 token 的 PROPFIND 必须 403");
        assert_eq!(options_status, 403, "无 token 的 OPTIONS 必须 403");
    }

    /// 错误 token（含只差一位、以及 token 前缀）必须被拒
    #[test]
    fn webdav_rejects_wrong_token() {
        let srv = TestServer::start();
        let (status, _) = srv.call("GET", "/00000000000000000000000000000000/readme.txt");
        let (prefix_status, _) = srv.call(
            "GET",
            &format!("/{}/readme.txt", &srv.token[..srv.token.len() - 1]),
        );
        let (upper_status, _) =
            srv.call("GET", &format!("/{}/readme.txt", srv.token.to_uppercase()));
        srv.shutdown();
        assert_eq!(status, 403, "错误 token 必须 403");
        assert_eq!(prefix_status, 403, "token 前缀（少一位）必须 403");
        assert_eq!(upper_status, 403, "大小写不同的 token 必须 403");
    }

    /// Host 不是 127.0.0.1:<端口>/localhost:<端口>：即使 token 正确也拒绝。
    /// 这就是防 DNS rebinding 的那一步。
    #[test]
    fn webdav_rejects_foreign_host() {
        let srv = TestServer::start();
        let path = srv.path("readme.txt");
        let (evil, _) = srv.call_host("GET", &path, &format!("evil.example.com:{}", srv.port));
        let (rebound, _) = srv.call_host("GET", &path, &format!("attacker.test:{}", srv.port));
        let (wrong_port, _) = srv.call_host("GET", &path, "127.0.0.1:1");
        let (no_port, _) = srv.call_host("GET", &path, "127.0.0.1");
        let (lookalike, _) =
            srv.call_host("GET", &path, &format!("127.0.0.1.evil.com:{}", srv.port));
        srv.shutdown();
        assert_eq!(evil, 403, "外部域名 Host 必须 403");
        assert_eq!(rebound, 403, "DNS rebinding 的域名 Host 必须 403");
        assert_eq!(wrong_port, 403, "端口不符的 Host 必须 403");
        assert_eq!(no_port, 403, "缺少端口的 Host 必须 403");
        assert_eq!(lookalike, 403, "形似回环的域名 Host 必须 403");
    }

    /// localhost:<端口> 是允许的（Windows 客户端可能这样发）
    #[test]
    fn webdav_accepts_localhost_host() {
        let srv = TestServer::start();
        let (status, body) = srv.call_host(
            "GET",
            &srv.path("readme.txt"),
            &format!("localhost:{}", srv.port),
        );
        srv.shutdown();
        assert_eq!(status, 200);
        assert_eq!(body, "Hello VFS");
    }

    /// 不带 Host 头的请求一律拒绝
    #[test]
    fn webdav_rejects_missing_host_header() {
        let srv = TestServer::start();
        let req = format!(
            "GET {} HTTP/1.1\r\nConnection: close\r\n\r\n",
            srv.path("readme.txt")
        );
        let resp = srv.raw(&req);
        srv.shutdown();
        assert_eq!(status_of(&resp), 403, "无 Host 头必须 403");
    }

    // ===== 解析器加固 =====

    /// `..` 段（原样与百分号编码）必须显式拒绝，不能靠 VFS 恰好没有这种键
    #[test]
    fn webdav_rejects_dotdot_path() {
        let srv = TestServer::start();
        let (plain, _) = srv.call("GET", &format!("/{}/../readme.txt", srv.token));
        let (encoded, _) = srv.call("GET", &format!("/{}/%2e%2e/readme.txt", srv.token));
        let (embedded, _) = srv.call("GET", &format!("/{}/src/../../readme.txt", srv.token));
        let (trailing, _) = srv.call("GET", &format!("/{}/src/..", srv.token));
        srv.shutdown();
        assert_eq!(plain, 403, "`..` 段必须 403");
        assert_eq!(encoded, 403, "`%2e%2e` 也必须 403");
        assert_eq!(embedded, 403, "中间夹 `..` 也必须 403");
        assert_eq!(trailing, 403, "结尾 `..` 也必须 403");
    }

    /// NUL 字节（原样与百分号编码）必须拒绝
    #[test]
    fn webdav_rejects_nul_byte_path() {
        let srv = TestServer::start();
        let (encoded, _) = srv.call("GET", &format!("/{}/a%00b", srv.token));
        let raw_req = format!(
            "GET /{}/a\0b HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            srv.token, srv.port
        );
        let resp = srv.raw(&raw_req);
        srv.shutdown();
        assert_eq!(encoded, 403, "编码后的 NUL 必须 403");
        assert_eq!(status_of(&resp), 403, "原样 NUL 必须 403");
    }

    /// 超大 Content-Length 直接 413（只读服务不接收 body）
    #[test]
    fn webdav_rejects_oversized_content_length() {
        let srv = TestServer::start();
        let req = format!(
            "PROPFIND {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            srv.path(""),
            srv.port,
            MAX_CONTENT_LENGTH + 1
        );
        let resp = srv.raw(&req);
        srv.shutdown();
        assert_eq!(status_of(&resp), 413, "超大 Content-Length 必须 413");
    }

    /// PROPFIND 不存在的路径返回 404，而不是空 207
    #[test]
    fn webdav_propfind_missing_is_404() {
        let srv = TestServer::start();
        let (status, _) = srv.call("PROPFIND", &srv.path("no/such/dir"));
        srv.shutdown();
        assert_eq!(status, 404);
    }

    // ===== token / 比较 / 路径规范化 =====

    /// token 长度 32 个十六进制字符（128 位），且每次都不同
    #[test]
    fn random_token_is_long_hex_and_unique() {
        let a = random_token();
        let b = random_token();
        assert_eq!(a.len(), 32, "token 应为 32 个十六进制字符: {}", a);
        assert!(
            a.chars().all(|c| c.is_ascii_hexdigit()),
            "token 应为十六进制: {}",
            a
        );
        assert_ne!(a, b, "两次生成不应相同");
    }

    /// 每次挂载使用不同的 token
    #[test]
    fn each_mount_gets_a_fresh_token() {
        let s1 = TestServer::start();
        let s2 = TestServer::start();
        let (t1, t2) = (s1.token.clone(), s2.token.clone());
        let (p1, p2) = (s1.port, s2.port);
        s1.shutdown();
        s2.shutdown();
        assert_eq!(t1.len(), 32);
        assert_ne!(t1, t2);
        assert_ne!(p1, p2, "端口由系统分配，两次不应相同");
    }

    /// 用 A 服务的 token 访问 B 服务必须失败
    #[test]
    fn token_is_not_interchangeable_across_mounts() {
        let s1 = TestServer::start();
        let s2 = TestServer::start();
        let (status, _) = s2.call("GET", &format!("/{}/readme.txt", s1.token));
        let t1 = s1.token.clone();
        s1.shutdown();
        s2.shutdown();
        assert_eq!(status, 403, "别的挂载的 token 不应被接受（{}）", t1);
    }

    #[test]
    fn ct_eq_matches_only_identical_strings() {
        assert!(ct_eq("abc123", "abc123"));
        assert!(!ct_eq("abc123", "abc124"));
        assert!(!ct_eq("abc123", "abc12"));
        assert!(!ct_eq("", "a"));
        assert!(ct_eq("", ""));
    }

    #[test]
    fn host_is_allowed_only_loopback_with_service_port() {
        assert!(host_is_allowed(Some("127.0.0.1:8080"), 8080));
        assert!(host_is_allowed(Some("localhost:8080"), 8080));
        assert!(host_is_allowed(Some("LOCALHOST:8080"), 8080));
        assert!(!host_is_allowed(None, 8080));
        assert!(!host_is_allowed(Some("evil.com:8080"), 8080));
        assert!(!host_is_allowed(Some("127.0.0.1:9090"), 8080));
        assert!(!host_is_allowed(Some("127.0.0.1"), 8080));
        assert!(!host_is_allowed(Some("localhost"), 8080));
        assert!(!host_is_allowed(Some("[::1]:8080"), 8080));
        assert!(!host_is_allowed(Some("127.0.0.1.evil.com:8080"), 8080));
    }

    #[test]
    fn resolve_target_requires_token_and_rejects_traversal() {
        let token = "deadbeefdeadbeefdeadbeefdeadbeef";
        assert_eq!(
            resolve_target(&format!("/{}/readme.txt", token), token).as_deref(),
            Some("readme.txt")
        );
        assert_eq!(
            resolve_target(&format!("/{}/", token), token).as_deref(),
            Some("")
        );
        assert_eq!(resolve_target("/readme.txt", token), None);
        assert_eq!(resolve_target(&format!("/{}/../x", token), token), None);
        assert_eq!(resolve_target(&format!("/{}/%2e%2e/x", token), token), None);
        assert_eq!(resolve_target(&format!("/{}/a%00b", token), token), None);
        assert_eq!(resolve_target(&format!("/{}/x\0y", token), token), None);
    }

    /// 多字节 UTF-8 紧跟 `%` 不能让解析 panic（旧实现会切到非字符边界）
    #[test]
    fn percent_decode_does_not_panic_on_multibyte() {
        let _ = percent_decode("/%é");
        let _ = percent_decode("%é%");
        assert_eq!(percent_decode("/a%20b.txt"), "/a b.txt");
        assert_eq!(percent_decode("/%2e%2e"), "/..");
    }

    #[test]
    fn normalize_decodes_and_trims() {
        assert_eq!(normalize_path("/src/main.rs"), "src/main.rs");
        assert_eq!(normalize_path("/a%20b.txt"), "a b.txt");
        assert_eq!(normalize_path("/x?y=1"), "x");
    }

    /// 位图全满：没有任何空闲盘符
    #[test]
    fn pick_free_from_mask_all_used_is_none() {
        assert_eq!(pick_free_from_mask(0x03FF_FFFF, None), None);
    }

    /// 只有 A:/B: 被占：仍应从 Z: 开始给
    #[test]
    fn pick_free_from_mask_ignores_floppy_letters() {
        assert_eq!(pick_free_from_mask(0b11, None).as_deref(), Some("Z:"));
    }

    /// 从 Z: 向下找第一个空闲者
    #[test]
    fn pick_free_from_mask_searches_downward() {
        // A..Y 全占，只剩 Z
        assert_eq!(
            pick_free_from_mask(0x01FF_FFFC, None).as_deref(),
            Some("Z:")
        );
        // Z 也占了，退到 Y
        assert_eq!(
            pick_free_from_mask(0x02FF_FFFC, None).as_deref(),
            Some("Y:")
        );
    }

    /// 系统盘即使空闲也不返回
    #[test]
    fn pick_free_from_mask_never_returns_system_drive() {
        // D..Z 全占，C 空闲但它是系统盘，故无可用盘符
        assert_eq!(pick_free_from_mask(0x03FF_FFF8, Some(2)), None);
        // 反过来：只有 C 空闲且非系统盘时才给 C
        assert_eq!(
            pick_free_from_mask(0x03FF_FFF8, None).as_deref(),
            Some("C:")
        );
    }

    #[test]
    fn is_drive_letter_accepts_only_single_letter() {
        assert!(is_drive_letter("Z:"));
        assert!(is_drive_letter("c:"));
        assert!(!is_drive_letter(""));
        assert!(!is_drive_letter("Z"));
        assert!(!is_drive_letter("ZZ:"));
        assert!(!is_drive_letter("1:"));
    }

    /// 依赖本机盘符布局，但断言只要求「格式正确且不含禁用盘符」，
    /// 因此任何机器上都不会误报。
    #[test]
    fn pick_free_drive_is_well_formed() {
        match pick_free_drive() {
            None => {}
            Some(d) => {
                assert_eq!(d.len(), 2, "盘符应为字母加冒号: {}", d);
                let mut chars = d.chars();
                let letter = chars.next().unwrap();
                assert!(('C'..='Z').contains(&letter), "不应是 A:/B:: {}", d);
                assert_eq!(chars.next(), Some(':'), "盘符应以冒号结尾: {}", d);
                let sys = std::env::var("SystemDrive").unwrap_or_default();
                let sys = sys.trim().to_ascii_uppercase();
                assert_ne!(d.to_ascii_uppercase(), sys, "不应返回系统盘: {}", d);
            }
        }
    }

    /// 未建立映射的句柄卸载时不得触碰盘符：用非法盘符构造，
    /// 即使守卫失效，`net use` 也只会无害失败，不会动真实映射。
    #[test]
    fn unmount_without_mapping_does_not_touch_drive() {
        let h = MountHandle {
            stop: Arc::new(AtomicBool::new(false)),
            port: 0,
            token: "unused".to_string(),
            drive: "~~:".to_string(),
            mapped: false,
            server: None,
        };
        unmount(h);
    }

    /// `url()` 必须带上 token（否则调用方拿到的地址会被服务自己 403）
    #[test]
    fn mount_handle_url_contains_token() {
        let h = MountHandle {
            stop: Arc::new(AtomicBool::new(false)),
            port: 41234,
            token: "0123456789abcdef0123456789abcdef".to_string(),
            drive: String::new(),
            mapped: false,
            server: None,
        };
        assert_eq!(
            h.url(),
            "http://127.0.0.1:41234/0123456789abcdef0123456789abcdef/"
        );
    }

    #[test]
    fn mount_error_service_has_no_handle() {
        let e = MountError::Service("bind failed".to_string());
        assert!(e.into_handle().is_none());
    }
}

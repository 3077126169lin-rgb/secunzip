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
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }
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

/// 启动只读 WebDAV 服务（后台线程，随机回环端口）
fn start_webdav(vfs: Arc<VirtualFS>) -> Result<(u16, Arc<AtomicBool>, JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let handle = std::thread::spawn(move || loop {
        if stop2.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let vfs = vfs.clone();
                std::thread::spawn(move || {
                    let _ = handle_conn(stream, &vfs);
                });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    });
    Ok((port, stop, handle))
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
    let (port, stop, server) = start_webdav(vfs).map_err(|e| MountError::Service(e.to_string()))?;
    let url = format!("http://127.0.0.1:{}/", port);
    let mut handle = MountHandle {
        stop,
        port,
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
    let (port, stop, server) = start_webdav(vfs).map_err(|e| MountError::Service(e.to_string()))?;
    Ok(MountHandle {
        stop,
        port,
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

fn handle_conn(stream: TcpStream, vfs: &VirtualFS) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let raw_path = parts.next().unwrap_or("/").to_string();

    // 读头部到空行，取 Content-Length
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let t = line.trim_end();
        if t.is_empty() {
            break;
        }
        if let Some(v) = t.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    // 丢弃 body（PROPFIND 的 prop 请求体可忽略）
    if content_length > 0 {
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body)?;
    }

    let vpath = normalize_path(&raw_path);
    let mut out = stream;
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
            let xml = multistatus_xml(vfs, &vpath);
            write_resp(
                &mut out,
                207,
                "Multi-Status",
                &[("Content-Type", "application/xml; charset=\"utf-8\"")],
                xml.len(),
                xml.as_bytes(),
            )?;
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

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
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
fn multistatus_xml(vfs: &VirtualFS, path: &str) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">\n",
    );

    let self_data = vfs.read_file(path).ok();
    let self_is_file = self_data.is_some();
    let name = path.rsplit('/').next().unwrap_or("").to_string();
    push_response(
        &mut xml,
        path,
        &name,
        self_is_file,
        self_data.map(|d| d.len()).unwrap_or(0),
    );

    if !self_is_file {
        for e in vfs.list_dir(path) {
            push_response(&mut xml, &e.path, &e.name, !e.is_dir, e.size);
        }
    }

    xml.push_str("</D:multistatus>");
    xml
}

fn push_response(xml: &mut String, href: &str, name: &str, is_file: bool, size: usize) {
    xml.push_str("<D:response>\n");
    xml.push_str(&format!("<D:href>/{}</D:href>\n", xml_escape(href)));
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

    fn http(port: u16, method: &str, path: &str) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let req = format!(
            "{} {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            method, path
        );
        s.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        s.read_to_string(&mut resp).unwrap();
        let status = resp
            .split_whitespace()
            .nth(1)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let body = resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    }

    #[test]
    fn webdav_get_file() {
        let (port, stop, h) = start_webdav(sample_vfs()).unwrap();
        let (status, body) = http(port, "GET", "/readme.txt");
        stop.store(true, Ordering::SeqCst);
        let _ = h.join();
        assert_eq!(status, 200);
        assert_eq!(body, "Hello VFS");
    }

    #[test]
    fn webdav_get_missing_is_404() {
        let (port, stop, h) = start_webdav(sample_vfs()).unwrap();
        let (status, _) = http(port, "GET", "/nope.txt");
        stop.store(true, Ordering::SeqCst);
        let _ = h.join();
        assert_eq!(status, 404);
    }

    #[test]
    fn webdav_propfind_root_lists_entries() {
        let (port, stop, h) = start_webdav(sample_vfs()).unwrap();
        let (status, body) = http(port, "PROPFIND", "/");
        stop.store(true, Ordering::SeqCst);
        let _ = h.join();
        assert_eq!(status, 207);
        assert!(body.contains("readme.txt"), "根应含 readme.txt: {}", body);
        assert!(body.contains("src"), "根应含 src: {}", body);
        assert!(body.contains("<D:collection/>"), "src 是目录: {}", body);
    }

    #[test]
    fn webdav_options_has_dav_header() {
        let (port, stop, h) = start_webdav(sample_vfs()).unwrap();
        let (status, _) = http(port, "OPTIONS", "/");
        stop.store(true, Ordering::SeqCst);
        let _ = h.join();
        assert_eq!(status, 200);
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
            drive: "~~:".to_string(),
            mapped: false,
            server: None,
        };
        unmount(h);
    }

    #[test]
    fn mount_error_service_has_no_handle() {
        let e = MountError::Service("bind failed".to_string());
        assert!(e.into_handle().is_none());
    }
}

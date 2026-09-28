//! 内存只读虚拟盘挂载（WebDAV）
//!
//! 把解密后的 VirtualFS 以「只读 WebDAV」在本机回环地址服务出来，
//! 再用 Windows 自带的 WebClient（`net use`）映射成资源管理器里的驱动器。
//! 全程内存、只读、不落盘，卸载即释放，防止复制扩散。
//!
//! 说明：Windows 自带的 WebDAV 迷你重定向器对匿名/HTTP 偶有挑剔，
//! 故 `mount_vfs_to_drive` 在 `net use` 失败时仍返回服务句柄，
//! 调用方可用返回的 URL 手动「映射网络驱动器」。

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
    server: Option<JoinHandle<()>>,
}

impl MountHandle {
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn drive(&self) -> &str {
        &self.drive
    }
    /// 本机 WebDAV 地址（可手动「映射网络驱动器」）
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }
}

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

/// 把 VirtualFS 挂载为资源管理器里的驱动器（内存只读）
#[cfg(windows)]
pub fn mount_vfs_to_drive(vfs: Arc<VirtualFS>, drive: &str) -> Result<MountHandle> {
    let (port, stop, server) = start_webdav(vfs)?;
    let url = format!("http://127.0.0.1:{}/", port);
    // 用 Windows 自带 WebClient 映射为驱动器；/user 携带凭据以避免交互式提示。
    let _ = std::process::Command::new("net")
        .args(["use", drive, &url, "/user:guest", "guest", "/persistent:no"])
        .output();
    Ok(MountHandle {
        stop,
        port,
        drive: drive.to_string(),
        server: Some(server),
    })
}

/// 非 Windows：仅启动 WebDAV 服务（无驱动器映射）
#[cfg(not(windows))]
pub fn mount_vfs_to_drive(vfs: Arc<VirtualFS>, drive: &str) -> Result<MountHandle> {
    let (port, stop, server) = start_webdav(vfs)?;
    Ok(MountHandle {
        stop,
        port,
        drive: drive.to_string(),
        server: Some(server),
    })
}

/// 卸载：断开驱动器 + 停止服务
pub fn unmount(h: MountHandle) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("net")
            .args(["use", h.drive(), "/delete", "/y"])
            .output();
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
    let mut xml =
        String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\">\n");

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
    xml.push_str(&format!(
        "<D:href>/{}</D:href>\n",
        xml_escape(href)
    ));
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
    use std::io::Read as _;

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
}

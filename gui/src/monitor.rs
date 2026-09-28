//! 后台监控线程：服务器连通性 + 待审批自动拉取。

use crate::api;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) struct Monitor {
    pub(crate) url: Mutex<String>,
    pub(crate) server_up: AtomicU8,       // 0 未知 / 1 在线 / 2 离线
    pub(crate) auto_requests: AtomicBool, // 管理页可见时自动拉取待审批
    pub(crate) requests: Mutex<Vec<api::RequestInfo>>,
    pub(crate) app_id: Mutex<String>,
    pub(crate) secret: Mutex<String>,
}

impl Monitor {
    pub(crate) fn spawn() -> Arc<Monitor> {
        let m = Arc::new(Monitor {
            url: Mutex::new(String::new()),
            server_up: AtomicU8::new(0),
            auto_requests: AtomicBool::new(false),
            requests: Mutex::new(Vec::new()),
            app_id: Mutex::new(String::new()),
            secret: Mutex::new(String::new()),
        });
        let mm = m.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            loop {
                let url = mm.url.lock().unwrap().clone();
                if url.trim().is_empty() {
                    mm.server_up.store(0, Ordering::Relaxed);
                } else {
                    let up = rt.block_on(api::ping_server(&url));
                    mm.server_up
                        .store(if up { 1 } else { 2 }, Ordering::Relaxed);
                    if up && mm.auto_requests.load(Ordering::Relaxed) {
                        let app_id = mm.app_id.lock().unwrap().clone();
                        let secret = mm.secret.lock().unwrap().clone();
                        if !app_id.is_empty() && !secret.is_empty() {
                            if let Ok(list) =
                                rt.block_on(api::list_requests(&url, &app_id, &secret))
                            {
                                *mm.requests.lock().unwrap() = list;
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(4));
            }
        });
        m
    }
}

/// 检测本机局域网 IP（跨机部署：把可访问地址告知接收方）
pub(crate) fn local_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

/// 服务器地址是否只对本机可见（接收方无法连接）
pub(crate) fn is_loopback(server: &str) -> bool {
    let s = server.to_lowercase();
    s.contains("127.0.0.1") || s.contains("localhost") || s.contains("[::1]")
}

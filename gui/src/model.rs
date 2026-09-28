//! 应用状态枚举与本地持久化（配置、打包记录）。

use std::path::PathBuf;

#[derive(PartialEq, Clone, Copy)]
pub(crate) enum AppState {
    Setup,
    Main,
}

#[derive(PartialEq, Clone, Copy)]
pub(crate) enum Mode {
    Open,
    Pack,
    Settings,
}

#[derive(PartialEq, Clone, Copy)]
pub(crate) enum OpenTab {
    Browse,
    Manage,
    Mine,
}

pub(crate) enum Action {
    EnterDir(String),
    Preview(String),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct AppConfig {
    pub(crate) user_id: String,
    pub(crate) server_url: String,
}

impl AppConfig {
    pub(crate) fn config_path() -> PathBuf {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        home.join("Documents").join("SecUnzip").join("config.json")
    }

    pub(crate) fn legacy_config_path() -> PathBuf {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("SecUnzip")
            .join("config.json")
    }

    pub(crate) fn load() -> Option<AppConfig> {
        if let Ok(data) = std::fs::read_to_string(Self::config_path()) {
            if let Ok(c) = serde_json::from_str(&data) {
                return Some(c);
            }
        }
        if let Ok(data) = std::fs::read_to_string(Self::legacy_config_path()) {
            if let Ok(c) = serde_json::from_str::<AppConfig>(&data) {
                c.save();
                return Some(c);
            }
        }
        None
    }

    pub(crate) fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// 本客户端打包过的文件清单（本地记录，让 GUI 能列出/打开自己创建的文件）
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub(crate) struct PackedEntry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) app_id: String,
    pub(crate) secret: PathBuf,
    pub(crate) size: u64,
    pub(crate) created_at: String,
}

impl PackedEntry {
    pub(crate) fn registry_path() -> PathBuf {
        AppConfig::config_path().with_file_name("packed.json")
    }
    pub(crate) fn load_all() -> Vec<PackedEntry> {
        std::fs::read_to_string(Self::registry_path())
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    }
    pub(crate) fn save_all(list: &[PackedEntry]) {
        let path = Self::registry_path();
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        if let Ok(j) = serde_json::to_string_pretty(list) {
            let _ = std::fs::write(path, j);
        }
    }
    pub(crate) fn add(entry: PackedEntry) {
        let mut list = Self::load_all();
        list.retain(|e| e.path != entry.path);
        list.insert(0, entry);
        Self::save_all(&list);
    }
    pub(crate) fn remove(path: &std::path::Path) {
        let mut list = Self::load_all();
        list.retain(|e| e.path != path);
        Self::save_all(&list);
    }
}

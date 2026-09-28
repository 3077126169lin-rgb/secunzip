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
    /// 勾选「记住口令」且取密钥成功后才会写入；旧配置文件没有这一项，按空列表读入
    #[serde(default)]
    pub(crate) remembered_passwords: Vec<RememberedPassword>,
    /// 记住的管理口令（grant / approve 专用，与取密钥口令分字段存放）。
    ///
    /// 这个字段由命令行侧写入同一份 config.json；GUI 读取后必须在保存时原样写回，
    /// 否则 serde 反序列化虽不报错，但一次 save() 就会把它整段抹掉，
    /// 命令行那边「记住管理口令」随之失效。同样明文保存，不做加密。
    #[serde(default)]
    pub(crate) admin_password: Option<String>,
    /// 其它客户端（命令行）写进同一份 config.json 的顶层字段，原样保留。
    ///
    /// serde 默认会忽略未知字段，反序列化不报错，但 GUI 一 save() 就会把它们全部丢掉——
    /// 这正是 admin_password 曾经失效的原因。这里用 flatten 兜住所有未知字段，
    /// 以后命令行再加字段也不会被 GUI 的保存抹掉。
    #[serde(flatten)]
    pub(crate) extra: serde_json::Map<String, serde_json::Value>,
}

/// 本机记住的取密钥口令。
///
/// 明文保存：config.json 本来就以明文存放个人 ID 与服务器地址，这里同样不做任何加密，
/// 也不假装它是加密存储——它只是一份本机文件，请自行保护好用户目录。
/// 只在勾选「记住口令」且取密钥成功后才写入；取密钥失败绝不写。
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub(crate) struct RememberedPassword {
    /// 文件 ID（.secunzip 的 md5），优先按「文件 + 用户」精确匹配
    pub(crate) app_id: String,
    /// 服务器地址，用于回退到「同一服务器 + 同一用户」匹配
    pub(crate) server_url: String,
    pub(crate) user_id: String,
    pub(crate) password: String,
}

impl RememberedPassword {
    /// 查找可自动填入的口令：先「文件 + 用户」精确匹配，再退到「服务器 + 用户」。
    pub(crate) fn find<'a>(
        list: &'a [RememberedPassword],
        app_id: &str,
        server_url: &str,
        user_id: &str,
    ) -> Option<&'a RememberedPassword> {
        list.iter()
            .find(|e| {
                !e.password.is_empty()
                    && !app_id.is_empty()
                    && e.app_id == app_id
                    && e.user_id == user_id
            })
            .or_else(|| {
                list.iter().find(|e| {
                    !e.password.is_empty()
                        && !server_url.is_empty()
                        && e.server_url == server_url
                        && e.user_id == user_id
                })
            })
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(app_id: &str, server_url: &str, user_id: &str, password: &str) -> RememberedPassword {
        RememberedPassword {
            app_id: app_id.into(),
            server_url: server_url.into(),
            user_id: user_id.into(),
            password: password.into(),
        }
    }

    /// 先按「文件 + 用户」精确匹配，再回退到「同一服务器 + 同一用户」，别的都不匹配
    #[test]
    fn remembered_password_prefers_file_then_server() {
        let list = vec![
            entry("a1", "http://s", "u1", "p1"),
            entry("a2", "http://s", "u1", "p2"),
        ];
        assert_eq!(
            RememberedPassword::find(&list, "a2", "http://s", "u1")
                .unwrap()
                .password,
            "p2"
        );
        // 这个文件没记过：退到同一服务器 + 同一用户
        assert_eq!(
            RememberedPassword::find(&list, "a9", "http://s", "u1")
                .unwrap()
                .password,
            "p1"
        );
        assert!(RememberedPassword::find(&list, "a9", "http://s", "u2").is_none());
        assert!(RememberedPassword::find(&list, "a9", "http://other", "u1").is_none());
        // 空口令条目不算「记住」，不能自动填入
        let blank = vec![entry("a1", "http://s", "u1", "")];
        assert!(RememberedPassword::find(&blank, "a1", "http://s", "u1").is_none());
    }

    /// 旧配置文件（没有 remembered_passwords / admin_password 字段）必须照常读入，
    /// 新字段按明文回写后再读回不丢
    #[test]
    fn config_json_accepts_old_files_and_keeps_passwords() {
        let old: AppConfig =
            serde_json::from_str(r#"{"user_id":"u1","server_url":"http://s"}"#).unwrap();
        assert!(old.remembered_passwords.is_empty());
        assert_eq!(old.admin_password, None);

        let cfg = AppConfig {
            user_id: "u1".into(),
            server_url: "http://s".into(),
            remembered_passwords: vec![entry("a1", "http://s", "u1", "明文口令")],
            admin_password: Some("管理口令".into()),
            extra: serde_json::Map::new(),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        // 如实明文落盘（不做加密，也不假装加密）
        assert!(json.contains("明文口令"));
        let back: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.user_id, "u1");
        assert_eq!(back.remembered_passwords.len(), 1);
        assert_eq!(back.remembered_passwords[0].password, "明文口令");
        assert_eq!(back.admin_password.as_deref(), Some("管理口令"));
    }

    /// 命令行写入的顶层 admin_password 不能被 GUI 的一次保存抹掉：
    /// 读入 → 原样写回 → 再读回，字段必须还在。
    #[test]
    fn config_json_preserves_cli_admin_password() {
        let from_cli: AppConfig = serde_json::from_str(
            r#"{"user_id":"u1","server_url":"http://s","admin_password":"cli 管理口令"}"#,
        )
        .unwrap();
        assert_eq!(from_cli.admin_password.as_deref(), Some("cli 管理口令"));

        let round_tripped: AppConfig =
            serde_json::from_str(&serde_json::to_string(&from_cli).unwrap()).unwrap();
        assert_eq!(
            round_tripped.admin_password.as_deref(),
            Some("cli 管理口令")
        );
    }

    /// 命令行以后再加的顶层字段也要原样保留（GUI 保存不再吞掉未知字段）
    #[test]
    fn config_json_preserves_unknown_cli_fields() {
        let from_cli: AppConfig = serde_json::from_str(
            r#"{"user_id":"u1","server_url":"http://s","admin_password":"p","future_field":{"a":1}}"#,
        )
        .unwrap();
        assert_eq!(from_cli.admin_password.as_deref(), Some("p"));
        assert_eq!(from_cli.extra.len(), 1);

        let json = serde_json::to_string(&from_cli).unwrap();
        let back: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.extra["future_field"]["a"], serde_json::json!(1));
        assert_eq!(back.admin_password.as_deref(), Some("p"));
    }
}

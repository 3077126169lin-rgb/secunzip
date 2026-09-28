//! `.secunzip` 文件类型注册（Windows）
//!
//! 把 .secunzip 扩展名关联到本程序（GUI），使用户双击文件即可用 SecUnzip 打开。
//! 写入 HKCU\Software\Classes（当前用户，无需管理员权限）。

/// 注册 .secunzip 文件类型关联到本 exe
#[cfg(windows)]
pub fn register_file_type() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_str = exe.display().to_string();
    let icon = format!("{},0", exe_str);
    let command = format!("\"{}\" \"%1\"", exe_str);

    // (注册表键, 默认值)
    let entries = [
        ("HKCU\\Software\\Classes\\.secunzip", "SecUnzip.File"),
        (
            "HKCU\\Software\\Classes\\SecUnzip.File",
            "SecUnzip 加密文件",
        ),
        (
            "HKCU\\Software\\Classes\\SecUnzip.File\\DefaultIcon",
            icon.as_str(),
        ),
        (
            "HKCU\\Software\\Classes\\SecUnzip.File\\shell\\open\\command",
            command.as_str(),
        ),
    ];

    for (key, value) in entries {
        let status = std::process::Command::new("reg")
            .args(["add", key, "/ve", "/d", value, "/f"])
            .status()
            .map_err(|e| format!("执行 reg 失败: {}", e))?;
        if !status.success() {
            return Err(format!("写入注册表失败: {}", key));
        }
    }

    // 通知资源管理器刷新关联（图标/右键）
    notify_shell_changed();

    Ok(format!("已注册 .secunzip 文件关联 → {}", exe_str))
}

/// 取消注册
#[cfg(windows)]
pub fn unregister_file_type() -> Result<String, String> {
    for key in [
        "HKCU\\Software\\Classes\\.secunzip",
        "HKCU\\Software\\Classes\\SecUnzip.File",
    ] {
        let _ = std::process::Command::new("reg")
            .args(["delete", key, "/f"])
            .status();
    }
    notify_shell_changed();
    Ok("已取消 .secunzip 文件关联".into())
}

/// 广播 shell 关联已变更
#[cfg(windows)]
fn notify_shell_changed() {
    // SHChangeNotify(SHCNE_ASSOCCHANGED=0x08000000, SHCNF_IDLIST=0, NULL, NULL)
    #[link(name = "shell32")]
    extern "system" {
        fn SHChangeNotify(wEventId: u32, uFlags: u32, dwItem1: *const u8, dwItem2: *const u8);
    }
    unsafe { SHChangeNotify(0x0800_0000, 0, std::ptr::null(), std::ptr::null()) };
}

// ===== 非 Windows 平台占位 =====

#[cfg(not(windows))]
pub fn register_file_type() -> Result<String, String> {
    Err("当前系统暂不支持文件类型注册（仅 Windows）".into())
}

#[cfg(not(windows))]
pub fn unregister_file_type() -> Result<String, String> {
    Err("当前系统暂不支持".into())
}

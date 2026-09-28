#![cfg_attr(windows, windows_subsystem = "windows")]
// Windows 下以 GUI 子系统构建，启动客户端时不弹出控制台窗口。
// --register / --unregister 由安装器或命令行调用时会接回父控制台输出（见 console_emit）。

mod api;
mod app;
mod icons;
mod model;
mod monitor;
mod register;
mod theme;
mod views;

fn main() -> eframe::Result {
    tracing_subscriber::fmt::init();

    // 命令行：--register / --unregister（安装器/设置面板调用）
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--register") {
        let msg = match register::register_file_type() {
            Ok(m) => m,
            Err(e) => format!("注册失败: {}", e),
        };
        console_emit(&msg);
        return Ok(());
    }
    if args.iter().any(|a| a == "--unregister") {
        let msg = match register::unregister_file_type() {
            Ok(m) => m,
            Err(e) => format!("取消注册失败: {}", e),
        };
        console_emit(&msg);
        return Ok(());
    }

    // 文件关联启动：双击 .secunzip 时带文件路径参数
    let initial_file: Option<std::path::PathBuf> = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .map(std::path::PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([600.0, 400.0])
            .with_clamp_size_to_monitor_size(true)
            .with_title("SecUnzip 客户端"),
        centered: true,
        // 不恢复上次的窗口尺寸/位置：一次异常退出会被永久沿用
        persist_window: false,
        ..Default::default()
    };

    eframe::run_native(
        "SecUnzip",
        options,
        Box::new(move |cc| {
            // 设置中文字体
            setup_custom_fonts(&cc.egui_ctx);
            Ok(Box::new(app::SecUnzipApp::new(initial_file)))
        }),
    )
}

fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // 尝试加载系统中文字体
    #[cfg(windows)]
    {
        let font_paths = [
            "C:\\Windows\\Fonts\\msyh.ttc",   // 微软雅黑
            "C:\\Windows\\Fonts\\simsun.ttc", // 宋体
            "C:\\Windows\\Fonts\\simhei.ttf", // 黑体
        ];

        for path in &font_paths {
            if let Ok(font_data) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert("chinese".to_owned(), egui::FontData::from_owned(font_data));
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .insert(0, "chinese".to_owned());
                fonts
                    .families
                    .entry(egui::FontFamily::Monospace)
                    .or_default()
                    .insert(0, "chinese".to_owned());
                break;
            }
        }
    }

    ctx.set_fonts(fonts);
}

/// 输出一行文本。
///
/// Windows 下本程序是 GUI 子系统（没有控制台）：
/// 若 stdout 是有效句柄（被管道/重定向，例如安装器捕获），直接写入；
/// 否则接回父进程的控制台再写 CONOUT$，保证命令行调用时结果可见。
fn console_emit(msg: &str) {
    use std::io::Write;
    let line = format!("{}\n", msg);
    if std::io::stdout().write_all(line.as_bytes()).is_ok() {
        let _ = std::io::stdout().flush();
        return;
    }
    write_to_parent_console(&line);
}

#[cfg(windows)]
fn write_to_parent_console(s: &str) {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(dwProcessId: u32) -> i32;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            sec: *mut core::ffi::c_void,
            disp: u32,
            flags: u32,
            tmpl: *mut core::ffi::c_void,
        ) -> *mut core::ffi::c_void;
        fn WriteFile(
            h: *mut core::ffi::c_void,
            buf: *const u8,
            n: u32,
            written: *mut u32,
            ov: *mut core::ffi::c_void,
        ) -> i32;
        fn CloseHandle(h: *mut core::ffi::c_void) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    const OPEN_EXISTING: u32 = 3;
    unsafe {
        // 父进程没有控制台（例如从资源管理器双击）时失败，放弃输出即可
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        let h = CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            SHARE_READ_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if h as isize == -1 {
            return;
        }
        let mut written = 0u32;
        let _ = WriteFile(
            h,
            s.as_ptr(),
            s.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        );
        let _ = CloseHandle(h);
    }
}

#[cfg(not(windows))]
fn write_to_parent_console(_s: &str) {}

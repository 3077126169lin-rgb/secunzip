//! 把 `assets/icon.ico` 作为资源嵌入可执行文件（仅 Windows）。
//!
//! 只使用工具链自带的资源编译器，不引入额外依赖：
//!   - GNU 目标（MinGW）：`windres`
//!   - MSVC 目标：`rc`
//!
//! 这两者默认都不在 PATH 上（rc 只在 VS 开发者命令提示符里，windres 随 MSYS2 提供），
//! 所以各自带一条常见安装路径的回退查找，否则会静默产出没有图标的可执行文件。
//!
//! 找不到图标或编译器时只打印警告、照常构建，不会让编译失败。

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // 从 crate 目录逐级向上找 assets/icon.ico（三个 crate 深度不同）
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let mut found: Option<PathBuf> = None;
    let mut dir = manifest.clone();
    for _ in 0..3 {
        let cand = dir.join("assets").join("icon.ico");
        if cand.is_file() {
            found = Some(cand);
            break;
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => break,
        }
    }

    let Some(icon) = found else {
        println!("cargo:warning=未找到 assets/icon.ico，跳过图标嵌入");
        return;
    };
    println!("cargo:rerun-if-changed={}", icon.display());

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());
    let rc_path = out_dir.join("app.rc");
    // .rc 里的路径需转义反斜杠
    let icon_escaped = icon.display().to_string().replace('\\', "\\\\");
    if let Err(e) = std::fs::write(&rc_path, format!("1 ICON \"{}\"\n", icon_escaped)) {
        println!("cargo:warning=写入 .rc 失败: {e}");
        return;
    }

    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let msvc = target_env == "msvc";
    let obj = out_dir.join(if msvc { "app.res" } else { "app.o" });

    let compiler = if msvc { find_rc() } else { find_windres() };
    let Some(compiler) = compiler else {
        println!(
            "cargo:warning=找不到资源编译器（{}），未嵌入图标",
            if msvc { "rc.exe" } else { "windres.exe" }
        );
        return;
    };

    let status = if msvc {
        Command::new(&compiler)
            .arg("/nologo")
            .arg(format!("/fo{}", obj.display()))
            .arg(&rc_path)
            .status()
    } else {
        Command::new(&compiler)
            .arg(&rc_path)
            .arg("-O")
            .arg("coff")
            .arg("-o")
            .arg(&obj)
            .status()
    };

    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg={}", obj.display());
        }
        Ok(s) => println!(
            "cargo:warning=资源编译未成功（退出码 {:?}），未嵌入图标",
            s.code()
        ),
        Err(e) => println!("cargo:warning=资源编译器执行失败（{e}），未嵌入图标"),
    }
}

/// 在 PATH 上查找可执行文件
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// 查找 rc.exe：先看 PATH，再扫 Windows Kits 下的各版本 SDK 目录。
fn find_rc() -> Option<PathBuf> {
    if let Some(p) = on_path("rc.exe") {
        return Some(p);
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    for key in ["ProgramFiles(x86)", "ProgramFiles"] {
        let Some(base) = std::env::var_os(key) else {
            continue;
        };
        let bin = PathBuf::from(base).join("Windows Kits").join("10").join("bin");
        let Ok(entries) = std::fs::read_dir(&bin) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path().join("x64").join("rc.exe");
            if p.is_file() {
                candidates.push(p);
            }
        }
    }
    // 目录名是 SDK 版本号，取最大的那个
    candidates.sort();
    candidates.pop()
}

/// 查找 windres.exe：先看 PATH，再试 MSYS2 的默认安装位置。
fn find_windres() -> Option<PathBuf> {
    if let Some(p) = on_path("windres.exe") {
        return Some(p);
    }
    for root in ["C:\\msys64", "C:\\msys32"] {
        for sub in [
            "mingw64\\bin",
            "ucrt64\\bin",
            "mingw32\\bin",
            "clang64\\bin",
        ] {
            let p = PathBuf::from(root).join(sub).join("windres.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

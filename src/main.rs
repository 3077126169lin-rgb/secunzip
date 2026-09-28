use clap::{CommandFactory, Parser};
use secunzip::cli::{args::Commands, commands, Cli};
use secunzip::runtime::selfextract::TRAILER_SIZE;
use secunzip::Result;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;

fn main() {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();

    // 黑盒自解压 EXE：runner 查自身尾部标记，命中则走黑盒打开流程（不经过 clap）。
    // 显式带子命令（pack/open/grant/...）时仍按普通 CLI 处理，工具自身功能不受影响。
    if !has_subcommand(&args) {
        if let Some((exe_path, exe)) = detect_self_extract_exe() {
            let user = flag_value(&args, &["--user", "-u"]);
            let output = flag_value(&args, &["--output", "-o"]).map(PathBuf::from);
            let password = flag_value(&args, &["--password"]);
            let remember = args.iter().skip(1).any(|a| a == "--remember");
            let result = commands::cmd_blackbox(&exe_path, &exe, user, output, password, remember);
            // 双击运行时 stdout 是终端：结束后等一次回车，避免窗口一闪而过
            finish(result, true);
            return;
        }
    }

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Pack {
            sources,
            output,
            server,
            allow_temp,
            blackbox,
        } => commands::cmd_pack(sources, output, server, allow_temp, blackbox),
        Commands::Open {
            file,
            user,
            output,
            password,
            remember,
        } => commands::cmd_open(file, user, output, password, remember),
        Commands::Grant {
            file,
            user,
            expires,
            password,
            remember,
        } => commands::cmd_grant(file, user, expires, password, remember),
        Commands::Revoke { file, user } => commands::cmd_revoke(file, user),
        Commands::Request {
            file,
            user,
            days,
            message,
            password,
            remember,
        } => commands::cmd_request(file, user, days, message, password, remember),
        Commands::Requests { file } => commands::cmd_requests(file),
        Commands::Approve {
            file,
            user,
            expires,
            password,
            remember,
        } => commands::cmd_approve(file, user, expires, password, remember),
        Commands::Deny { file, user } => commands::cmd_deny(file, user),
    };

    finish(result, false);
}

/// 参数中是否出现已知子命令
///
/// 用于区分「黑盒 EXE 被双击/直接运行」与「把工具本身当命令行用」。
fn has_subcommand(args: &[String]) -> bool {
    let cmd = Cli::command();
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .collect();
    args.iter().skip(1).any(|a| names.iter().any(|n| n == a))
}

/// 自查自身是否为黑盒自解压 EXE，命中则返回（自身路径, 自身字节）
///
/// 先只读末尾的尾部标记魔数做预筛，命中后才整读自身并交给
/// `runtime::detect_self_extract` 判定，避免每次 CLI 调用都整读一遍自身二进制。
fn detect_self_extract_exe() -> Option<(PathBuf, Vec<u8>)> {
    use std::io::{Read, Seek, SeekFrom};

    let path = std::env::current_exe().ok()?;
    let mut file = std::fs::File::open(&path).ok()?;
    if file.metadata().ok()?.len() < TRAILER_SIZE as u64 {
        return None;
    }
    file.seek(SeekFrom::End(-(TRAILER_SIZE as i64))).ok()?;
    let mut tail = [0u8; TRAILER_SIZE];
    file.read_exact(&mut tail).ok()?;
    // 尾部标记：[embedded_offset u64 LE][magic 8 字节]
    if &tail[8..16] != secunzip::MAGIC {
        return None;
    }
    drop(file);

    let bytes = std::fs::read(&path).ok()?;
    secunzip::runtime::detect_self_extract(&bytes).map(|_| (path, bytes))
}

/// 取 `--flag value` / `--flag=value` 的值（黑盒模式下的最小参数解析）
fn flag_value(args: &[String], names: &[&str]) -> Option<String> {
    let mut rest = args.iter().skip(1);
    while let Some(arg) = rest.next() {
        for name in names {
            if arg == name {
                return rest.next().cloned();
            }
            if let Some(v) = arg.strip_prefix(&format!("{}=", name)) {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// 统一收尾：错误打印到 stderr 并以 1 退出；pause 为真时等一次回车
fn finish(result: Result<()>, pause: bool) {
    if let Err(e) = result {
        eprintln!("错误: {}", e);
        if pause {
            wait_enter();
        }
        std::process::exit(1);
    }
    if pause {
        wait_enter();
    }
}

/// 仅当标准输出是终端（双击场景）时暂停；被管道/重定向调用时不阻塞
fn wait_enter() {
    if !std::io::stdout().is_terminal() {
        return;
    }
    print!("按回车键退出...");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}

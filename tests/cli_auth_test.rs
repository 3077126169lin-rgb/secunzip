//! CLI 口令与失败退出码：
//! 1. 各子命令都接受 `--password` / `--remember`，缺省为 None / false；
//! 2. 口令解析优先级 `--password` > `SECUNZIP_PASSWORD` > 配置记住的口令 > 标准输入（纯函数覆盖）；
//! 3. 申请天数只接受正整数；
//! 4. 用假本地 HTTP 服务端验证真正发出的报文：grant/request 带明文口令、
//!    approve 不带口令时整个字段都不下发、服务端 `success:false` 时命令返回 Err；
//! 5. 配置记住口令：只在成功后写入、按用途分字段、下次打开自动复用、被拒时给出可读提示。
//!
//! 涉及配置文件的用例一律以子进程方式跑 CLI，并把 USERPROFILE 指到临时目录，
//! 绝不触碰开发者本机真实的 config.json。

use clap::Parser;
use secunzip::cli::args::Commands;
use secunzip::cli::commands::{
    cmd_grant, cmd_request, pack_config, pick_optional_password, pick_password, validate_need_days,
    PasswordSource,
};
use secunzip::cli::Cli;
use secunzip::packer::PackBuilder;
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const KEY: &str = "CLI_AUTH_KEY_0123456789";
const SECRET: &str = "cli-auth-secret";
/// 取密钥成功的服务端应答（key 必须与打包时的 content_key 一致，才能真正解包）
const KEY_OK: &str = r#"{"success":true,"message":"成功","key":"CLI_AUTH_KEY_0123456789"}"#;

// ===== 1. clap 解析 =====

fn parse(extra: &[&str]) -> Commands {
    let mut args = vec!["secunzip", "grant", "a.secunzip", "--user", "u1"];
    args.extend_from_slice(extra);
    Cli::try_parse_from(args).unwrap().command
}

#[test]
fn grant_password_flag_defaults_to_none() {
    match parse(&[]) {
        Commands::Grant {
            password, remember, ..
        } => {
            assert!(password.is_none(), "未传 --password 应为 None");
            assert!(!remember, "未传 --remember 应为 false");
        }
        _ => panic!("应解析为 grant 子命令"),
    }
    // 空串由运行期判定为「未提供」，clap 层只负责收下
    match parse(&["--password", ""]) {
        Commands::Grant { password, .. } => assert_eq!(password.as_deref(), Some("")),
        _ => panic!("应解析为 grant 子命令"),
    }
}

#[test]
fn all_password_subcommands_accept_flag() {
    let with = |args: Vec<&str>| Cli::try_parse_from(args).unwrap().command;

    match with(vec![
        "secunzip",
        "open",
        "a.secunzip",
        "--user",
        "u1",
        "--password",
        "p1",
        "--remember",
    ]) {
        Commands::Open {
            password, remember, ..
        } => {
            assert_eq!(password.as_deref(), Some("p1"));
            assert!(remember);
        }
        _ => panic!("应解析为 open 子命令"),
    }
    match with(vec![
        "secunzip",
        "request",
        "a.secunzip",
        "--user",
        "u1",
        "--days",
        "3",
    ]) {
        Commands::Request {
            password,
            days,
            remember,
            ..
        } => {
            assert!(password.is_none());
            assert_eq!(days, Some(3));
            assert!(!remember);
        }
        _ => panic!("应解析为 request 子命令"),
    }
    match with(vec![
        "secunzip",
        "approve",
        "a.secunzip",
        "--user",
        "u1",
        "--password",
        "p2",
        "--remember",
    ]) {
        Commands::Approve {
            password, remember, ..
        } => {
            assert_eq!(password.as_deref(), Some("p2"));
            assert!(remember);
        }
        _ => panic!("应解析为 approve 子命令"),
    }
}

// ===== 2. 口令优先级 =====

#[test]
fn password_precedence_arg_env_config_stdin() {
    let pick = |a, e, c, s| pick_password(a, e, c, s).unwrap();
    let src = |a, e, c, s| pick_password(a, e, c, s).unwrap().source;

    let p = pick(
        Some("参数".into()),
        Some("环境".into()),
        Some("配置".into()),
        Some("输入".into()),
    );
    assert_eq!(p.value, "参数");
    assert_eq!(p.source, PasswordSource::Arg);

    assert_eq!(
        src(
            None,
            Some("环境".into()),
            Some("配置".into()),
            Some("输入".into())
        ),
        PasswordSource::Env
    );
    assert_eq!(
        src(None, None, Some("配置".into()), Some("输入".into())),
        PasswordSource::Config
    );
    assert_eq!(
        src(None, None, None, Some("输入".into())),
        PasswordSource::Stdin
    );
}

#[test]
fn blank_password_falls_through_and_missing_password_errors() {
    // 空串/纯空白不算提供，逐级回退
    let picked = pick_password(Some("   ".into()), None, None, Some("来自输入".into())).unwrap();
    assert_eq!(picked.value, "来自输入");

    let err = pick_password(Some("".into()), Some("\t".into()), None, None).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("缺少口令"), "应报中文缺口令错误: {}", msg);
    assert!(msg.contains("--password") && msg.contains("SECUNZIP_PASSWORD"));
}

#[test]
fn approve_password_only_from_explicit_input() {
    assert!(
        pick_optional_password(None, None).is_none(),
        "管理员不填则不下发"
    );
    let from_env = pick_optional_password(None, Some("env".into())).unwrap();
    assert_eq!(from_env.value, "env");
    assert_eq!(from_env.source, PasswordSource::Env);
    let arg_wins = pick_optional_password(Some("arg".into()), Some("env".into())).unwrap();
    assert_eq!(arg_wins.value, "arg");
    assert_eq!(arg_wins.source, PasswordSource::Arg);
    assert!(pick_optional_password(Some("  ".into()), None).is_none());
}

// ===== 3. 申请天数 =====

#[test]
fn need_days_must_be_positive() {
    assert!(validate_need_days(None).is_ok());
    assert!(validate_need_days(Some(1)).is_ok());
    assert!(validate_need_days(Some(365)).is_ok());
    for bad in [0, -1, -30] {
        let err = validate_need_days(Some(bad)).unwrap_err().to_string();
        assert!(err.contains("正整数"), "{} 应被拒: {}", bad, err);
    }
}

// ===== 4. 假服务端：真实报文与失败传播 =====

/// 极简 HTTP 服务端：顺序处理 `times` 个请求，每次回同一段 JSON，并把请求体回传
fn serve(times: usize, response: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for _ in 0..times {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            let mut body_start = None;
            let mut content_len = 0usize;
            loop {
                let n = match stream.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                buf.extend_from_slice(&tmp[..n]);
                if body_start.is_none() {
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        body_start = Some(pos + 4);
                        let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                        for line in head.lines() {
                            if let Some(v) = line.strip_prefix("content-length:") {
                                content_len = v.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                }
                if let Some(s) = body_start {
                    if buf.len() >= s + content_len {
                        break;
                    }
                }
            }
            let body = body_start
                .map(|s| String::from_utf8_lossy(&buf[s..]).to_string())
                .unwrap_or_default();
            let _ = tx.send(body);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.len(),
                response
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    (addr, rx)
}

/// 造一个指向假服务端的 .secunzip 与配套 .secret
fn make_pack(server: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("a.txt"), "内容").unwrap();
    let out = dir.path().join("app.secunzip");
    let config = pack_config(server.into(), KEY.into(), true, false);
    PackBuilder::new(config, vec![src], out.clone())
        .build()
        .unwrap();
    fs::write(out.with_extension("secret"), SECRET).unwrap();
    (dir, out)
}

fn body_of(rx: &mpsc::Receiver<String>) -> serde_json::Value {
    serde_json::from_str(&rx.recv_timeout(Duration::from_secs(10)).unwrap()).unwrap()
}

#[test]
fn grant_sends_plaintext_password_and_refusal_is_err() {
    let (addr, rx) = serve(1, r#"{"success":false,"message":"密钥错误"}"#);
    let (_dir, file) = make_pack(&addr);

    let err = cmd_grant(file, "u1".into(), None, Some("明文口令".into()), false).unwrap_err();
    assert!(
        err.to_string().contains("密钥错误"),
        "服务端拒绝必须变成 Err 并原样带出 message: {}",
        err
    );

    let body = body_of(&rx);
    assert_eq!(body["password"], json!("明文口令"), "口令必须原样明文传输");
    assert_eq!(body["user_id"], json!("u1"));
}

#[test]
fn grant_success_returns_ok() {
    let (addr, rx) = serve(1, r#"{"success":true,"message":"已授权 u1"}"#);
    let (_dir, file) = make_pack(&addr);

    cmd_grant(
        file,
        "u1".into(),
        Some("7d".into()),
        Some("p".into()),
        false,
    )
    .unwrap();
    let body = body_of(&rx);
    assert_eq!(body["password"], json!("p"));
    assert_eq!(body["expires_at"], json!("7d"));
}

#[test]
fn request_sends_password_and_positive_days() {
    let (addr, rx) = serve(1, r#"{"success":true,"message":"已提交申请"}"#);
    let (_dir, file) = make_pack(&addr);

    cmd_request(
        file,
        "u1".into(),
        Some(3),
        Some("说明".into()),
        Some("我的口令".into()),
        false,
    )
    .unwrap();
    let body = body_of(&rx);
    assert_eq!(body["password"], json!("我的口令"));
    assert_eq!(body["need_days"], json!(3));
}

#[test]
fn request_rejects_non_positive_days_before_any_io() {
    // 文件根本不存在也不该先报文件错：天数校验发生在最前面
    let err = cmd_request(
        PathBuf::from("不存在的文件.secunzip"),
        "u1".into(),
        Some(0),
        None,
        Some("p".into()),
        false,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("正整数"), "0 天必须在本地被拒: {}", err);
}

// ===== 5. 子进程：退出码与配置文件 =====

/// 以临时 USERPROFILE 跑真 CLI，避免碰到开发机真实的 config.json
fn cli(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_secunzip"));
    cmd.env("USERPROFILE", home)
        .env("APPDATA", home.join("appdata"))
        .env_remove("SECUNZIP_PASSWORD")
        .env_remove("SECUNZIP_USER")
        .env_remove("SECUNZIP_SECRET")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn run(cmd: &mut Command) -> (bool, String, String) {
    let out = cmd.output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn read_config(home: &Path) -> serde_json::Value {
    let path = home.join("Documents").join("SecUnzip").join("config.json");
    serde_json::from_str(&fs::read_to_string(&path).expect("配置应已写入")).unwrap()
}

/// 端到端：被服务端拒绝时进程必须以非 0 退出，脚本查 $? / $LASTEXITCODE 才有意义
#[test]
fn failed_grant_exits_nonzero() {
    let home = tempfile::tempdir().unwrap();
    let (addr, _rx) = serve(1, r#"{"success":false,"message":"密钥错误"}"#);
    let (_dir, file) = make_pack(&addr);

    let (ok, stdout, stderr) = run(cli(home.path()).args([
        "grant",
        file.to_str().unwrap(),
        "--user",
        "u1",
        "--password",
        "p",
    ]));
    assert!(
        !ok,
        "服务端拒绝必须让进程非 0 退出，stdout={} stderr={}",
        stdout, stderr
    );
    assert!(stderr.contains("密钥错误"), "stderr 必须带出服务端 message");
}

#[test]
fn approve_omits_password_when_admin_gives_none() {
    let home = tempfile::tempdir().unwrap();
    // 关键场景：配置里已经记住了一个管理口令（grant --remember 留下的），
    // approve 不带 --password 时**必须**仍然不下发该字段，否则会静默覆盖申请者自己设的口令。
    let config_dir = home.path().join("Documents").join("SecUnzip");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.json"),
        serde_json::to_string_pretty(&json!({
            "user_id": "admin",
            "server_url": "http://127.0.0.1:1",
            "admin_password": "记住的管理口令",
        }))
        .unwrap(),
    )
    .unwrap();
    let before = fs::read_to_string(config_dir.join("config.json")).unwrap();

    let (addr, rx) = serve(2, r#"{"success":true,"message":"已通过"}"#);
    let (_dir, file) = make_pack(&addr);

    let (ok, out, err) =
        run(cli(home.path()).args(["approve", file.to_str().unwrap(), "--user", "u1"]));
    assert!(ok, "审批应成功: {}", err);
    let body = body_of(&rx);
    assert!(
        body.get("password").is_none(),
        "管理员不填口令时不应下发该字段（更不能用配置里记住的口令）: {}",
        body
    );
    assert!(
        out.contains("沿用申请者设定的口令"),
        "应当明确提示留空 = 沿用申请口令: {}",
        out
    );
    assert_eq!(
        fs::read_to_string(config_dir.join("config.json")).unwrap(),
        before,
        "留空审批不应改动配置"
    );

    // 显式给了口令就必须下发
    let (ok, _out, err) = run(cli(home.path()).args([
        "approve",
        file.to_str().unwrap(),
        "--user",
        "u1",
        "--password",
        "管理员口令",
    ]));
    assert!(ok, "审批应成功: {}", err);
    assert_eq!(body_of(&rx)["password"], json!("管理员口令"));
}

#[test]
fn approve_remembers_only_explicitly_typed_password() {
    let home = tempfile::tempdir().unwrap();
    let (addr, _rx) = serve(1, r#"{"success":true,"message":"已通过"}"#);
    let (_dir, file) = make_pack(&addr);

    // 显式输入 + --remember → 记进 admin_password，供以后的 grant 用
    let (ok, _out, err) = run(cli(home.path()).args([
        "approve",
        file.to_str().unwrap(),
        "--user",
        "u1",
        "--password",
        "管理员口令",
        "--remember",
    ]));
    assert!(ok, "审批应成功: {}", err);
    let cfg = read_config(home.path());
    assert_eq!(cfg["admin_password"], json!("管理员口令"));
    assert!(
        cfg.get("remembered_passwords").is_none(),
        "管理口令不得混进取密钥口令列表"
    );
}

#[test]
fn open_remembers_password_and_reuses_it() {
    let home = tempfile::tempdir().unwrap();
    let (addr, rx) = serve(2, KEY_OK);
    let (_dir, file) = make_pack(&addr);
    let app_id = secunzip::crypto::md5_hex(&fs::read(&file).unwrap());
    let out1 = home.path().join("out1");

    // 第一次：显式口令 + --remember
    let (ok, stdout, err) = run(cli(home.path()).args([
        "open",
        file.to_str().unwrap(),
        "--user",
        "u1",
        "--password",
        "记住我",
        "--remember",
        "--output",
        out1.to_str().unwrap(),
    ]));
    assert!(ok, "首次打开应成功: {} {}", stdout, err);
    assert_eq!(body_of(&rx)["password"], json!("记住我"));

    // 写入的必须是 GUI 同构的 remembered_passwords，且不得混入管理口令
    let cfg = read_config(home.path());
    assert_eq!(cfg["user_id"], json!("u1"));
    assert_eq!(cfg["remembered_passwords"][0]["password"], json!("记住我"));
    assert_eq!(cfg["remembered_passwords"][0]["user_id"], json!("u1"));
    assert_eq!(cfg["remembered_passwords"][0]["app_id"], json!(app_id));
    assert_eq!(cfg["remembered_passwords"][0]["server_url"], json!(addr));
    assert!(
        cfg.get("admin_password").is_none(),
        "取密钥口令绝不能写进管理口令字段"
    );
    assert!(out1.join("a.txt").exists(), "应已解压落盘");

    // 第二次：不给 --password、无环境变量、stdin 为空，应从配置取到同一口令
    let out2 = home.path().join("out2");
    let (ok, stdout, err) = run(cli(home.path()).args([
        "open",
        file.to_str().unwrap(),
        "--user",
        "u1",
        "--output",
        out2.to_str().unwrap(),
    ]));
    assert!(ok, "二次打开应直接复用记住的口令: {} {}", stdout, err);
    assert_eq!(body_of(&rx)["password"], json!("记住我"));
}

#[test]
fn remembered_password_rejected_prints_actionable_hint() {
    // 手工造一份「记住的旧口令」配置，再让服务端以「口令错误」拒绝
    let h = tempfile::tempdir().unwrap();
    let (addr, rx) = serve(1, r#"{"success":false,"message":"口令错误"}"#);
    let (_dir, file) = make_pack(&addr);
    let config_dir = h.path().join("Documents").join("SecUnzip");
    fs::create_dir_all(&config_dir).unwrap();
    let original = serde_json::to_string_pretty(&json!({
        "user_id": "u1",
        "server_url": addr,
        "remembered_passwords": [{
            "app_id": secunzip::crypto::md5_hex(&fs::read(&file).unwrap()),
            "server_url": addr,
            "user_id": "u1",
            "password": "被改过的旧口令",
        }],
    }))
    .unwrap();
    fs::write(config_dir.join("config.json"), &original).unwrap();

    let (ok, stdout, err) = run(cli(h.path()).args([
        "open",
        file.to_str().unwrap(),
        "--user",
        "u1",
        "--output",
        h.path().join("out").to_str().unwrap(),
    ]));
    assert!(!ok, "口令被拒必须失败");
    assert_eq!(body_of(&rx)["password"], json!("被改过的旧口令"));

    let shown = format!("{}{}", stdout, err);
    assert!(
        shown.contains("口令错误"),
        "必须透出服务端 message: {}",
        shown
    );
    assert!(
        shown.contains("--password") && shown.contains("记住"),
        "必须提示可用 --password 覆盖并更新记住的口令: {}",
        shown
    );
    // 失败路径不写配置
    assert_eq!(
        fs::read_to_string(config_dir.join("config.json")).unwrap(),
        original,
        "取密钥失败时不得改动配置文件"
    );
}

#[test]
fn grant_remembers_admin_password_separately() {
    let home = tempfile::tempdir().unwrap();
    let (addr, rx) = serve(2, r#"{"success":true,"message":"已授权 u1"}"#);
    let (_dir, file) = make_pack(&addr);

    let (ok, _out, err) = run(cli(home.path()).args([
        "grant",
        file.to_str().unwrap(),
        "--user",
        "u1",
        "--password",
        "管理口令",
        "--remember",
    ]));
    assert!(ok, "授权应成功: {}", err);
    assert_eq!(body_of(&rx)["password"], json!("管理口令"));

    let cfg = read_config(home.path());
    assert_eq!(cfg["admin_password"], json!("管理口令"));
    assert!(
        cfg.get("remembered_passwords").is_none(),
        "管理口令不得混进取密钥口令列表"
    );

    // 第二次不传口令：应从 admin_password 取到
    let (ok, _out, err) =
        run(cli(home.path()).args(["grant", file.to_str().unwrap(), "--user", "u2"]));
    assert!(ok, "二次授权应复用记住的管理口令: {}", err);
    assert_eq!(body_of(&rx)["password"], json!("管理口令"));
}

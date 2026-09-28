//! CLI 黑盒自解压 EXE：
//! 1. `pack --blackbox` 能被 clap 解析（默认关闭）；
//! 2. `--blackbox` 走 CLI 自己的打包配置时，产物确实带自解压尾部标记、
//!    内嵌部分是标准 .secunzip（可被 CLI 的 open 流程内存载入）；
//! 3. 不加 `--blackbox` 的 .secunzip 产物不会被误判为黑盒。
//!
//! 注：`pack_file` 的注册步骤需要真实服务端，这里只覆盖到「配置 → 产物 → 检测」链路。

use clap::Parser;
use secunzip::cli::args::Commands;
use secunzip::cli::commands::pack_config;
use secunzip::cli::Cli;
use secunzip::core::OutputFormat;
use secunzip::packer::PackBuilder;
use secunzip::runtime::{detect_self_extract, embedded_secunzip, RuntimeLoader};
use std::fs;

const SERVER: &str = "http://127.0.0.1:1";
const KEY: &str = "CLI_BLACKBOX_KEY_0123456789";

fn parse_pack(extra: &[&str]) -> bool {
    let mut args = vec![
        "secunzip", "pack", "src", "--output", "out.bin", "--server", SERVER,
    ];
    args.extend_from_slice(extra);
    match Cli::try_parse_from(args).unwrap().command {
        Commands::Pack { blackbox, .. } => blackbox,
        _ => panic!("应解析为 pack 子命令"),
    }
}

#[test]
fn pack_blackbox_flag_parsed() {
    assert!(!parse_pack(&[]), "未传 --blackbox 时应为 false");
    assert!(parse_pack(&["--blackbox"]), "--blackbox 应为 true");
}

#[test]
fn cli_blackbox_exe_embeds_detectable_secunzip() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("a.txt"), "CLI 黑盒内容 42").unwrap();

    // 与 CLI `pack --blackbox` 完全相同的配置路径
    let config = pack_config(SERVER.into(), KEY.into(), true, true);
    assert_eq!(config.format, OutputFormat::Exe);

    let output = temp_dir.path().join("box.exe");
    PackBuilder::new(config, vec![source_dir], output.clone())
        .build()
        .unwrap();
    let bytes = fs::read(&output).unwrap();

    // 1. 尾部标记可被 main 的检测入口识别
    let offset = detect_self_extract(&bytes).expect("黑盒 EXE 应能被 detect_self_extract 识别");
    assert!(offset < bytes.len(), "内嵌偏移必须落在文件内");

    // 2. 内嵌部分是标准 .secunzip：头部格式为 SecUnzip，可直接喂给 RuntimeLoader
    let embedded = embedded_secunzip(&bytes).unwrap();
    let loader = RuntimeLoader::from_bytes(embedded).unwrap();
    assert_eq!(*loader.format(), OutputFormat::SecUnzip);
    let vfs = loader.mount_vfs_with_key(KEY).unwrap();
    assert_eq!(vfs.read_text("a.txt").unwrap(), "CLI 黑盒内容 42");

    // 3. 文件 ID 口径一致：黑盒用 EXE 自身 MD5，与 pack_file 的 file_md5_hex 相同
    assert_eq!(
        secunzip::crypto::md5_hex(&bytes),
        secunzip::crypto::file_md5_hex(&output).unwrap()
    );
}

#[test]
fn cli_secunzip_artifact_is_not_detected_as_blackbox() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("a.txt"), "普通产物").unwrap();

    // blackbox = false → 自有 .secunzip 格式，不应被当成自解压黑盒
    let config = pack_config(SERVER.into(), KEY.into(), true, false);
    assert_eq!(config.format, OutputFormat::SecUnzip);

    let output = temp_dir.path().join("plain.secunzip");
    PackBuilder::new(config, vec![source_dir], output.clone())
        .build()
        .unwrap();
    let bytes = fs::read(&output).unwrap();

    assert!(detect_self_extract(&bytes).is_none());
    assert!(embedded_secunzip(&bytes).is_err());
}

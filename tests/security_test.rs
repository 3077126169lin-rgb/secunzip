//! 端到端安全验证：
//! 1. content_key / 明文绝不写入打包文件（防泄漏）
//! 2. 注册服务端失败时绝不留下「死文件」（防孤儿不可开文件）

use secunzip::core::*;
use secunzip::packer::PackBuilder;
use std::fs;

fn config_with_key(key: &str) -> PackConfig {
    PackConfig {
        format: OutputFormat::SecUnzip,
        compress: CompressAlgo::Zip,
        crypto: CryptoAlgo::Aes256Cbc,
        hash: HashAlgo::Sha256,
        key_derive: KeyNode::Input(KeySource::Literal(key.to_string())),
        run_mode: RunMode::TempDir,
        auth_mode: AuthMode::Remote("http://127.0.0.1:1".into()),
        expire_at: None,
        ip_whitelist: Vec::new(),
        salt: b"test salt value 32 bytes long!!".to_vec(),
        app_id: None,
        allow_temp: false,
    }
}

#[test]
fn content_key_and_plaintext_never_in_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    let secret_plaintext = "TOP-SECRET-CONTENT-DO-NOT-LEAK-12345";
    fs::write(source_dir.join("secret.txt"), secret_plaintext).unwrap();

    // 用唯一且足够长的 content_key，避免偶然字节碰撞
    let content_key = "MY_CONTENT_KEY_abcdef0123456789_F3X";
    let output = temp_dir.path().join("o.secunzip");
    let builder = PackBuilder::new(
        config_with_key(content_key),
        vec![source_dir],
        output.clone(),
    );
    builder.build().unwrap();
    let bytes = fs::read(&output).unwrap();

    // content_key 明文绝不出现（服务端才持有）
    assert!(
        !bytes
            .windows(content_key.len())
            .any(|w| w == content_key.as_bytes()),
        "content_key 泄漏进了打包文件字节"
    );
    // 原文绝不出现（已加密）
    assert!(
        !bytes
            .windows(secret_plaintext.len())
            .any(|w| w == secret_plaintext.as_bytes()),
        "明文泄漏进了打包文件字节"
    );
}

#[test]
fn register_failure_leaves_no_dead_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("a.txt"), "hello").unwrap();
    let output = temp_dir.path().join("out.secunzip");

    // 不可达服务端（127.0.0.1:1 连接被拒）→ 注册必失败
    let result = secunzip::cli::commands::pack_file(
        vec![source_dir],
        output.clone(),
        "http://127.0.0.1:1".into(),
        true,
        None,
        false,
    );

    assert!(result.is_err(), "注册失败应返回错误，而非静默产出死文件");
    assert!(!output.exists(), "注册失败后不应留下 .secunzip 死文件");

    // 也不应留下 .secret 或任何打包产物
    let leftovers: Vec<String> = fs::read_dir(temp_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".secret") || n.ends_with(".secunzip"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "注册失败后不应留下任何产物: {:?}",
        leftovers
    );
}

#[test]
fn blackbox_exe_self_extract_roundtrip() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("a.txt"), "blackbox content 42").unwrap();

    let content_key = "BLACKBOX_KEY_0123456789abcdef";
    let mut config = config_with_key(content_key);
    config.format = OutputFormat::Exe; // 黑盒 EXE 输出
    let output = temp_dir.path().join("box.exe");
    let builder = PackBuilder::new(config, vec![source_dir], output.clone());
    builder.build().unwrap();
    let bytes = fs::read(&output).unwrap();

    // 检测自解压尾部标记 + 取出内嵌的标准 .secunzip
    let embedded =
        secunzip::runtime::embedded_secunzip(&bytes).expect("黑盒 EXE 应含自解压内嵌数据");

    // 内存解压（黑盒）：用 content_key 挂载 VFS
    let loader = secunzip::runtime::RuntimeLoader::from_bytes(embedded).unwrap();
    let vfs = loader.mount_vfs_with_key(content_key).unwrap();
    assert_eq!(vfs.read_text("a.txt").unwrap(), "blackbox content 42");

    // content_key 不应泄漏进内嵌 .secunzip（存根是工具自身二进制，另测）
    assert!(
        !embedded
            .windows(content_key.len())
            .any(|w| w == content_key.as_bytes()),
        "content_key 泄漏进内嵌 .secunzip"
    );
}

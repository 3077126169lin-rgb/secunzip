//! Zip Slip 端到端验证。
//!
//! 打包器（`add_dir_to_zip`）只会用真实相对路径做条目名，伪造产物才可能含 `../`。
//! 这里直接用 `zip` crate 手工构造恶意 ZIP，再按 loader 期望的格式加密、组装头部，
//! 走完整的 `RuntimeLoader::from_bytes` → `extract_to_dir` 路径，断言：
//! 1. 解压失败并报出点名条目的中文错误；
//! 2. 目标目录之外不产生任何文件（也不留下半个目标目录）。

use secunzip::core::*;
use secunzip::crypto::{create_encryptor, keywrap::derive_key_iv};
use secunzip::key_derive::KeyDeriveEngine;
use secunzip::runtime::RuntimeLoader;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

const TEST_KEY: &str = "zip-slip-e2e-test-key";

/// 用 zip crate 直接构造 ZIP（条目名原样写入，不做任何清洗）
fn zip_with_entry(entry: &str, content: &[u8]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(entry, options).unwrap();
        zip.write_all(content).unwrap();
        zip.finish().unwrap();
    }
    buf.into_inner()
}

/// 按 loader 期望的格式组装产物：[头部][密文]，密钥来自 Literal(TEST_KEY)
fn build_artifact(zip_bytes: &[u8]) -> Vec<u8> {
    let config = PackConfig {
        format: OutputFormat::SecUnzip,
        compress: CompressAlgo::Zip,
        crypto: CryptoAlgo::Aes256Gcm,
        hash: HashAlgo::Sha256,
        key_derive: KeyNode::Input(KeySource::Literal(TEST_KEY.to_string())),
        run_mode: RunMode::TempDir,
        auth_mode: AuthMode::Local,
        expire_at: None,
        ip_whitelist: Vec::new(),
        salt: b"zip slip salt 32 bytes long!!!!!".to_vec(),
        app_id: None,
        allow_temp: false,
    };

    let engine = KeyDeriveEngine::new(config.salt.clone());
    let key_material = engine.derive(&config.key_derive).unwrap();
    let encryptor = create_encryptor(&config.crypto);
    let (key, iv) = derive_key_iv(
        &key_material,
        &config.salt,
        encryptor.key_len(),
        encryptor.iv_len(),
    );
    let encrypted = encryptor.encrypt(zip_bytes, &key, &iv).unwrap();
    let integrity_hash = Sha256::digest(zip_bytes).to_vec();

    let mut header = PackHeader {
        magic: *secunzip::MAGIC,
        version: secunzip::FORMAT_VERSION,
        format: OutputFormat::SecUnzip,
        config,
        data_offset: 0, // 稍后按实际头部长度回填
        data_size: encrypted.len() as u64,
        original_size: zip_bytes.len() as u64,
        integrity_hash,
    };
    let header_bytes = header.to_bytes().unwrap();
    header.data_offset = header_bytes.len() as u64;
    let header_bytes = header.to_bytes().unwrap();

    let mut artifact = header_bytes;
    artifact.extend_from_slice(&encrypted);
    artifact
}

/// 递归收集目录下所有文件的相对路径
fn collect_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    out.sort();
    out
}

/// 正常产物仍必须能正常解压（校验不能误伤合法条目名）
#[test]
fn normal_artifact_still_extracts() {
    let temp = tempfile::tempdir().unwrap();
    let dest = temp.path().join("dest");

    let zip_bytes = {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("hello.txt", options).unwrap();
            zip.write_all(b"Hello").unwrap();
            zip.start_file("sub\\nested.txt", options).unwrap();
            zip.write_all(b"Nested").unwrap();
            zip.finish().unwrap();
        }
        buf.into_inner()
    };

    let loader = RuntimeLoader::from_bytes(&build_artifact(&zip_bytes)).unwrap();
    loader.extract_to_dir(&dest).unwrap();

    assert_eq!(
        std::fs::read_to_string(dest.join("hello.txt")).unwrap(),
        "Hello"
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("sub").join("nested.txt")).unwrap(),
        "Nested"
    );
}

#[test]
fn extract_to_dir_rejects_zip_slip_entry() {
    let temp = tempfile::tempdir().unwrap();
    let dest = temp.path().join("dest");
    // dest.join("../escape.txt") 会落到 temp/escape.txt
    let outside = temp.path().join("escape.txt");

    let artifact = build_artifact(&zip_with_entry("../escape.txt", b"PWNED"));
    let loader = RuntimeLoader::from_bytes(&artifact).unwrap();

    let err = loader.extract_to_dir(&dest).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("../escape.txt"),
        "错误必须点名恶意条目，实际: {}",
        msg
    );
    assert!(
        msg.contains("上级目录"),
        "错误必须说明拒绝原因，实际: {}",
        msg
    );

    assert!(
        !outside.exists(),
        "目标目录之外被写入: {}",
        outside.display()
    );
    assert!(!dest.exists(), "校验失败时不应留下目标目录");
    // 临时目录下除产物本身外不应有任何落盘内容
    assert!(
        collect_files(temp.path()).is_empty(),
        "解压被拒后不应有任何文件: {:?}",
        collect_files(temp.path())
    );
}

#[test]
fn extract_with_key_rejects_zip_slip_entry() {
    // CLI `open -o <目录>` 不走 extract_to_dir，而是直接 dest.join(name)；
    // 它走的是 extract_with_key → decrypt_and_decompress，这里证明同一道校验
    // 也在那条路径上生效（虽未改动 cli/commands.rs，该路径同样不会写出目录之外）。
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("escape.txt");

    let artifact = build_artifact(&zip_with_entry("..\\..\\escape.txt", b"PWNED"));
    let loader = RuntimeLoader::from_bytes(&artifact).unwrap();

    let err = loader
        .extract_with_key(TEST_KEY)
        .err()
        .expect("含 .. 段的条目必须被拒绝");
    let msg = err.to_string();
    assert!(
        msg.contains("..\\..\\escape.txt"),
        "错误必须点名恶意条目，实际: {}",
        msg
    );

    assert!(
        !outside.exists(),
        "目标目录之外被写入: {}",
        outside.display()
    );
}

#[test]
fn extract_with_key_rejects_absolute_entry() {
    let artifact = build_artifact(&zip_with_entry("C:\\Windows\\evil.dll", b"PWNED"));
    let loader = RuntimeLoader::from_bytes(&artifact).unwrap();

    let err = loader
        .extract_with_key(TEST_KEY)
        .err()
        .expect("盘符前缀条目必须被拒绝");
    assert!(err.to_string().contains("盘符前缀"), "实际错误: {}", err);
}

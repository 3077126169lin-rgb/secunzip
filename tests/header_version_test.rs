//! FORMAT_VERSION 校验：头部版本与当前构建不一致时必须在解析入口明确报错，
//! 而不是用今天的规则去硬解未来版本的文件。
//!
//! 头部布局固定：magic(8) 之后紧接 version(2, 小端)，因此可以直接定位并改写版本字段。
//! 测试走 `RuntimeLoader::from_bytes`，与 loader 实际使用的解析路径完全一致。

use secunzip::core::*;
use secunzip::packer::PackBuilder;
use secunzip::runtime::RuntimeLoader;

/// 头部中版本字段的偏移（magic 之后）
const VERSION_OFFSET: usize = 8;

fn sample_artifact() -> Vec<u8> {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(source_dir.join("a.txt"), "version check content").unwrap();

    let config = PackConfig {
        format: OutputFormat::SecUnzip,
        compress: CompressAlgo::Zip,
        crypto: CryptoAlgo::Aes256Gcm,
        hash: HashAlgo::Sha256,
        key_derive: KeyNode::Input(KeySource::Literal("version-key".into())),
        run_mode: RunMode::TempDir,
        auth_mode: AuthMode::Local,
        expire_at: None,
        ip_whitelist: Vec::new(),
        salt: b"version check salt 32 bytes!!!".to_vec(),
        app_id: None,
        allow_temp: false,
    };
    let output = temp_dir.path().join("o.secunzip");
    PackBuilder::new(config, vec![source_dir], output.clone())
        .build()
        .unwrap();
    std::fs::read(&output).unwrap()
}

/// 写入头部版本字段，返回改写后的字节
fn with_version(bytes: &[u8], version: u16) -> Vec<u8> {
    let mut patched = bytes.to_vec();
    patched[VERSION_OFFSET..VERSION_OFFSET + 2].copy_from_slice(&version.to_le_bytes());
    patched
}

#[test]
fn artifact_of_current_version_parses() {
    // 先确认样本本身可用，否则下面的用例会因为样本坏了而失去意义
    let bytes = sample_artifact();
    let loader = RuntimeLoader::from_bytes(&bytes).expect("当前版本的产物应能解析");
    assert_eq!(loader.header().version, secunzip::FORMAT_VERSION);
}

#[test]
fn future_version_is_rejected_with_both_numbers() {
    let mut bytes = sample_artifact();
    // 正常产物可解析 → 只改版本字段
    RuntimeLoader::from_bytes(&bytes).expect("改版本前应能解析");
    bytes[VERSION_OFFSET..VERSION_OFFSET + 2]
        .copy_from_slice(&(secunzip::FORMAT_VERSION + 1).to_le_bytes());

    // 只能返回错误，不能 panic，也不能被当成合法产物接受
    let err = RuntimeLoader::from_bytes(&bytes)
        .err()
        .expect("未来版本必须被拒绝");
    let msg = err.to_string();
    let future = secunzip::FORMAT_VERSION + 1;
    assert!(
        msg.contains(&format!("版本 {}", future)),
        "错误应写明文件版本 {}，实际: {}",
        future,
        msg
    );
    assert!(
        msg.contains(&format!("版本 {}", secunzip::FORMAT_VERSION)),
        "错误应写明本程序支持的版本 {}，实际: {}",
        secunzip::FORMAT_VERSION,
        msg
    );
    assert!(
        msg.contains("无法读取"),
        "错误应说明当前程序无法读取，实际: {}",
        msg
    );
}

#[test]
fn older_version_is_rejected() {
    // 版本字段由 builder 用 FORMAT_VERSION 写入，不存在更旧的合法产物；
    // 更小的值只能说明文件被伪造或损坏，同样明确报错。
    let bytes = with_version(&sample_artifact(), secunzip::FORMAT_VERSION - 1);

    let err = RuntimeLoader::from_bytes(&bytes)
        .err()
        .expect("更低的版本必须被拒绝");
    let msg = err.to_string();
    assert!(
        msg.contains(&format!(
            "版本 {}",
            secunzip::FORMAT_VERSION.saturating_sub(1)
        )),
        "错误应写明文件版本，实际: {}",
        msg
    );
    assert!(
        msg.contains("低于"),
        "错误应说明版本低于本程序支持，实际: {}",
        msg
    );
}

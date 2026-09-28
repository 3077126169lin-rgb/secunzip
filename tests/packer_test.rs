use std::fs;
use std::path::PathBuf;
use secunzip::core::*;
use secunzip::packer::PackBuilder;
use secunzip::runtime::RuntimeLoader;

fn create_test_files(dir: &PathBuf) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("hello.txt"), "Hello, World!").unwrap();
    fs::write(dir.join("data.json"), r#"{"key": "value"}"#).unwrap();
    
    let sub_dir = dir.join("subdir");
    fs::create_dir_all(&sub_dir).unwrap();
    fs::write(sub_dir.join("nested.txt"), "Nested file content").unwrap();
}

fn default_config() -> PackConfig {
    PackConfig {
        format: OutputFormat::SecUnzip,
        compress: CompressAlgo::Zip,
        crypto: CryptoAlgo::Aes256Cbc,
        hash: HashAlgo::Sha256,
        key_derive: KeyNode::Input(KeySource::Literal("test key".into())),
        run_mode: RunMode::TempDir,
        auth_mode: AuthMode::Local,
        expire_at: None,
        ip_whitelist: Vec::new(),
        salt: b"test salt value 32 bytes long!!".to_vec(),
        app_id: None,
        allow_temp: false,
    }
}

#[test]
fn test_pack_and_unpack_secunzip() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let output_file = temp_dir.path().join("test.secunzip");
    let extract_dir = temp_dir.path().join("extracted");
    
    create_test_files(&source_dir);
    
    let config = default_config();
    let builder = PackBuilder::new(config, vec![source_dir.clone()], output_file.clone());
    builder.build().unwrap();
    
    assert!(output_file.exists());
    
    let loader = RuntimeLoader::from_file(&output_file).unwrap();
    loader.extract_to_dir(&extract_dir).unwrap();
    
    assert!(extract_dir.join("hello.txt").exists());
    assert!(extract_dir.join("data.json").exists());
    assert!(extract_dir.join("subdir").join("nested.txt").exists());
    
    let content = fs::read_to_string(extract_dir.join("hello.txt")).unwrap();
    assert_eq!(content, "Hello, World!");
}

#[test]
fn test_pack_with_expiry() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let output_file = temp_dir.path().join("test.secunzip");
    
    create_test_files(&source_dir);
    
    let yesterday = (chrono::Local::now() - chrono::Duration::days(1))
        .format("%Y%m%d")
        .to_string();
    
    let mut config = default_config();
    config.expire_at = Some(yesterday);
    
    let builder = PackBuilder::new(config, vec![source_dir], output_file.clone());
    builder.build().unwrap();
    
    let loader = RuntimeLoader::from_file(&output_file).unwrap();
    let result = loader.extract_to_dir(&temp_dir.path().join("extract"));
    assert!(result.is_err());
}

#[test]
fn test_pack_with_chacha20() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let output_file = temp_dir.path().join("test.secunzip");
    let extract_dir = temp_dir.path().join("extracted");
    
    create_test_files(&source_dir);
    
    let mut config = default_config();
    config.crypto = CryptoAlgo::ChaCha20;
    config.key_derive = KeyNode::Input(KeySource::Literal("chacha key".into()));
    config.salt = b"chacha salt 32 bytes long!!!!!!".to_vec();
    
    let builder = PackBuilder::new(config, vec![source_dir], output_file.clone());
    builder.build().unwrap();
    
    let loader = RuntimeLoader::from_file(&output_file).unwrap();
    loader.extract_to_dir(&extract_dir).unwrap();
    
    assert!(extract_dir.join("hello.txt").exists());
}

#[test]
fn test_wrong_key_cannot_decrypt() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let output_file = temp_dir.path().join("test.secunzip");
    
    create_test_files(&source_dir);
    
    let config = default_config();
    let builder = PackBuilder::new(config, vec![source_dir], output_file.clone());
    builder.build().unwrap();
    
    let mut data = fs::read(&output_file).unwrap();
    
    let salt_marker = b"salt";
    for i in 0..data.len() - salt_marker.len() {
        if &data[i..i + salt_marker.len()] == salt_marker {
            data[i..i + salt_marker.len()].copy_from_slice(b"WRNG");
            break;
        }
    }
    
    let loader = RuntimeLoader::from_bytes(&data).unwrap();
    let result = loader.extract_to_dir(&temp_dir.path().join("extract"));
    assert!(result.is_err());
}

#[test]
fn test_extract_with_server_key() {
    // 模拟服务端下发密钥联动：打包用 Literal(file_key)，打开用 extract_with_key(file_key)
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let output_file = temp_dir.path().join("test.secunzip");

    create_test_files(&source_dir);

    let file_key = "server-distributed-content-key-0001";
    let mut config = default_config();
    config.key_derive = KeyNode::Input(KeySource::Literal(file_key.to_string()));

    let builder = PackBuilder::new(config, vec![source_dir], output_file.clone());
    builder.build().unwrap();

    // 用服务端下发的密钥挂载 VFS（内存，不落盘）
    let loader = RuntimeLoader::from_file(&output_file).unwrap();
    let vfs = loader.mount_vfs_with_key(file_key).unwrap();

    assert_eq!(vfs.file_count(), 3);
    assert_eq!(vfs.read_text("hello.txt").unwrap(), "Hello, World!");
    assert_eq!(vfs.read_text("subdir/nested.txt").unwrap(), "Nested file content");

    // 错误密钥应失败
    assert!(loader.mount_vfs_with_key("wrong-key").is_err());
}

#[test]
fn test_info_command() {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let output_file = temp_dir.path().join("test.secunzip");
    
    create_test_files(&source_dir);
    
    let mut config = default_config();
    config.expire_at = Some("20261231".into());
    config.ip_whitelist = vec!["192.168.1.0/24".into()];
    config.app_id = Some("test-app-id".into());
    config.allow_temp = true;
    
    let builder = PackBuilder::new(config, vec![source_dir], output_file.clone());
    builder.build().unwrap();
    
    let loader = RuntimeLoader::from_file(&output_file).unwrap();
    let header = loader.header();
    
    assert_eq!(header.version, 1);
    assert_eq!(header.config.expire_at, Some("20261231".into()));
    assert_eq!(header.config.ip_whitelist, vec!["192.168.1.0/24"]);
    assert_eq!(header.config.app_id, Some("test-app-id".into()));
    assert!(header.config.allow_temp);
}

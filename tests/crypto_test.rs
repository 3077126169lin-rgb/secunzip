use secunzip::core::{
    AuthMode, CompressAlgo, CryptoAlgo, HashAlgo, KeyNode, KeySource, OutputFormat, PackConfig,
    RunMode,
};
use secunzip::crypto::{create_encryptor, hash::create_hasher, keywrap::derive_key_iv};
use secunzip::packer::PackBuilder;
use secunzip::runtime::RuntimeLoader;

#[test]
fn test_aes256_encrypt_decrypt() {
    let encryptor = create_encryptor(&CryptoAlgo::Aes256Cbc);

    let key = vec![0x42u8; 32]; // 32字节密钥
    let iv = vec![0x24u8; 16]; // 16字节IV
    let plaintext = b"Hello, SecUnzip! This is a test message.";

    // 加密
    let ciphertext = encryptor.encrypt(plaintext, &key, &iv).unwrap();
    assert_ne!(ciphertext, plaintext);

    // 解密
    let decrypted = encryptor.decrypt(&ciphertext, &key, &iv).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_chacha20_encrypt_decrypt() {
    let encryptor = create_encryptor(&CryptoAlgo::ChaCha20);

    let key = vec![0x42u8; 32]; // 32字节密钥
    let iv = vec![0x24u8; 12]; // 12字节nonce
    let plaintext = b"Hello, SecUnzip! ChaCha20 test.";

    // 加密
    let ciphertext = encryptor.encrypt(plaintext, &key, &iv).unwrap();
    assert_ne!(ciphertext, plaintext);

    // 解密
    let decrypted = encryptor.decrypt(&ciphertext, &key, &iv).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_wrong_key_fails() {
    let encryptor = create_encryptor(&CryptoAlgo::Aes256Cbc);

    let key1 = vec![0x42u8; 32];
    let key2 = vec![0x43u8; 32];
    let iv = vec![0x24u8; 16];
    let plaintext = b"Secret data";

    let ciphertext = encryptor.encrypt(plaintext, &key1, &iv).unwrap();

    // 用错误密钥解密应该失败
    let result = encryptor.decrypt(&ciphertext, &key2, &iv);
    assert!(result.is_err());
}

#[test]
fn test_aes256_gcm_roundtrip() {
    let encryptor = create_encryptor(&CryptoAlgo::Aes256Gcm);

    let key = vec![0x42u8; 32];
    let nonce = vec![0x24u8; 12]; // GCM 用 12 字节 nonce
    let plaintext = b"Hello, SecUnzip! GCM authenticated encryption.";

    let ciphertext = encryptor.encrypt(plaintext, &key, &nonce).unwrap();
    assert_ne!(ciphertext, plaintext);
    // 密文 = 明文 + 16 字节认证 tag
    assert_eq!(ciphertext.len(), plaintext.len() + 16);

    let decrypted = encryptor.decrypt(&ciphertext, &key, &nonce).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_aes256_gcm_detects_tampering() {
    let encryptor = create_encryptor(&CryptoAlgo::Aes256Gcm);

    let key = vec![0x42u8; 32];
    let nonce = vec![0x24u8; 12];
    let plaintext = b"tamper me please";

    // 翻转密文中间一个比特：CBC 会静默产出错误明文，GCM 必须拒绝
    let mut ct = encryptor.encrypt(plaintext, &key, &nonce).unwrap();
    let mid = ct.len() / 2;
    ct[mid] ^= 0x01;
    assert!(
        encryptor.decrypt(&ct, &key, &nonce).is_err(),
        "GCM 应检测出密文被篡改"
    );

    // 篡改认证 tag 同样必须失败
    let mut ct2 = encryptor.encrypt(plaintext, &key, &nonce).unwrap();
    let last = ct2.len() - 1;
    ct2[last] ^= 0x01;
    assert!(
        encryptor.decrypt(&ct2, &key, &nonce).is_err(),
        "GCM 应检测出 tag 被篡改"
    );
}

#[test]
fn test_aes256_gcm_wrong_key_fails() {
    let encryptor = create_encryptor(&CryptoAlgo::Aes256Gcm);
    let key1 = vec![0x42u8; 32];
    let key2 = vec![0x43u8; 32];
    let nonce = vec![0x24u8; 12];

    let ct = encryptor.encrypt(b"secret data", &key1, &nonce).unwrap();
    assert!(encryptor.decrypt(&ct, &key2, &nonce).is_err());
}

#[test]
fn test_hash_algorithms() {
    let data = b"test data for hashing";

    // SHA-256
    let hasher = create_hasher(&HashAlgo::Sha256).unwrap();
    let hash = hasher.hash(data);
    assert_eq!(hash.len(), 32);

    // SHA-512
    let hasher = create_hasher(&HashAlgo::Sha512).unwrap();
    let hash = hasher.hash(data);
    assert_eq!(hash.len(), 64);

    // BLAKE3
    let hasher = create_hasher(&HashAlgo::Blake3).unwrap();
    let hash = hasher.hash(data);
    assert_eq!(hash.len(), 32);
}

#[test]
fn test_hash_deterministic() {
    let data = b"consistent data";
    let hasher = create_hasher(&HashAlgo::Sha256).unwrap();

    let hash1 = hasher.hash(data);
    let hash2 = hasher.hash(data);

    assert_eq!(hash1, hash2);
}

#[test]
fn test_key_derivation() {
    let material = b"my secret material";
    let salt = b"random salt";

    let (key, iv) = derive_key_iv(material, salt, 32, 16);

    assert_eq!(key.len(), 32);
    assert_eq!(iv.len(), 16);

    // 相同输入应该产生相同输出
    let (key2, iv2) = derive_key_iv(material, salt, 32, 16);
    assert_eq!(key, key2);
    assert_eq!(iv, iv2);
}

#[test]
fn test_key_derivation_different_salt() {
    let material = b"my secret material";
    let salt1 = b"salt one";
    let salt2 = b"salt two";

    let (key1, _) = derive_key_iv(material, salt1, 32, 16);
    let (key2, _) = derive_key_iv(material, salt2, 32, 16);

    // 不同盐应该产生不同密钥
    assert_ne!(key1, key2);
}

#[test]
fn test_xchacha20_encrypt_decrypt() {
    let encryptor = create_encryptor(&CryptoAlgo::XChaCha20);

    let key = vec![0x42u8; 32]; // 32字节密钥
    let iv = vec![0x24u8; 24]; // 派生的 24 字节 nonce，全部直接使用
    let plaintext = b"Hello, SecUnzip! XChaCha20 test.";

    let ciphertext = encryptor.encrypt(plaintext, &key, &iv).unwrap();
    assert_ne!(ciphertext, plaintext);

    let decrypted = encryptor.decrypt(&ciphertext, &key, &iv).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn test_encryptor_iv_key_lengths() {
    // XChaCha20 必须使用原生 24 字节 nonce；其余算法长度保持不变
    assert_eq!(create_encryptor(&CryptoAlgo::XChaCha20).iv_len(), 24);
    assert_eq!(create_encryptor(&CryptoAlgo::XChaCha20).key_len(), 32);
    assert_eq!(create_encryptor(&CryptoAlgo::ChaCha20).iv_len(), 12);
    assert_eq!(create_encryptor(&CryptoAlgo::ChaCha20).key_len(), 32);
    assert_eq!(create_encryptor(&CryptoAlgo::Aes256Gcm).iv_len(), 12);
    assert_eq!(create_encryptor(&CryptoAlgo::Aes256Gcm).key_len(), 32);
    // AES-256-CBC 的 IV 是 16 字节，不属于本次改动范围
    assert_eq!(create_encryptor(&CryptoAlgo::Aes256Cbc).iv_len(), 16);
    assert_eq!(create_encryptor(&CryptoAlgo::Aes256Cbc).key_len(), 32);
}

#[test]
fn test_xchacha20_uses_full_24_byte_nonce() {
    // 24 字节全部进入 XNonce：同样的 24 字节必然得到同样的密文，
    // 且与同 key 同 iv 的 ChaCha20（96 位 nonce）结果不同，证明确实走了扩展 nonce 变体。
    let x = create_encryptor(&CryptoAlgo::XChaCha20);
    let c = create_encryptor(&CryptoAlgo::ChaCha20);

    let key = vec![0x11u8; 32];
    let iv = vec![0x22u8; 24];
    let plaintext = b"nonce mapping";

    let a = x.encrypt(plaintext, &key, &iv).unwrap();
    let b = x.encrypt(plaintext, &key, &iv).unwrap();
    assert_eq!(a, b, "同一 nonce 必须得到相同密文");

    let chacha = c.encrypt(plaintext, &key, &iv[..12]).unwrap();
    assert_ne!(a, chacha, "XChaCha20 不能退化成 ChaCha20");

    // 另一组 nonce 必须给出不同密文
    let other = x.encrypt(plaintext, &key, &[0x23u8; 24]).unwrap();
    assert_ne!(a, other);
}

#[test]
fn test_xchacha20_no_zero_padding_of_nonce() {
    // 旧构造把 iv[0..12] 放到前 12 字节、后 12 字节补零，
    // 因此前 12 字节相同、后 12 字节不同的两个 nonce 会给出相同密文。
    // 现在 24 字节全部使用，二者必须不同 —— 这是补零已被移除的最强证据。
    let x = create_encryptor(&CryptoAlgo::XChaCha20);
    let key = vec![0x11u8; 32];
    let plaintext = b"zero padding must be gone";

    let mut iv_a = vec![0x55u8; 24];
    let mut iv_b = vec![0x55u8; 24];
    iv_b[12..].copy_from_slice(&[0xAAu8; 12]);

    let a = x.encrypt(plaintext, &key, &iv_a).unwrap();
    let b = x.encrypt(plaintext, &key, &iv_b).unwrap();
    assert_ne!(
        a, b,
        "前 12 字节相同、后 12 字节不同的 nonce 必须产生不同密文（后 12 字节未被忽略）"
    );

    // 后 12 字节补零的 nonce 也不等于前 12 字节非零、后 12 字节全零的取值组合
    iv_a[12..].copy_from_slice(&[0u8; 12]);
    let zero_tail = x.encrypt(plaintext, &key, &iv_a).unwrap();
    let mut zero_tail_ref = vec![0u8; 24];
    zero_tail_ref[..12].copy_from_slice(&[0x55u8; 12]);
    assert_eq!(
        zero_tail,
        x.encrypt(plaintext, &key, &zero_tail_ref).unwrap()
    );
}

#[test]
fn test_xchacha20_rejects_wrong_nonce_len() {
    let encryptor = create_encryptor(&CryptoAlgo::XChaCha20);
    let key = vec![0x42u8; 32];

    // 12 字节曾是旧构造接受的派生长度，现在必须被拒绝
    assert!(
        encryptor.encrypt(b"data", &key, &[0u8; 12]).is_err(),
        "12 字节 IV 必须被拒绝，XChaCha20 使用 24 字节 nonce"
    );
    assert!(encryptor.encrypt(b"data", &key, &[0u8; 16]).is_err());
    assert!(encryptor.encrypt(b"data", &key, &[0u8; 32]).is_err());

    // 长度校验在解密路径同样生效
    assert!(encryptor.decrypt(b"data", &key, &[0u8; 12]).is_err());
}

#[test]
fn test_xchacha20_rejects_wrong_key_len() {
    let encryptor = create_encryptor(&CryptoAlgo::XChaCha20);
    let iv = vec![0x42u8; 24];

    assert!(encryptor.encrypt(b"data", &[0u8; 16], &iv).is_err());
    assert!(encryptor.encrypt(b"data", &[0u8; 31], &iv).is_err());
    assert!(encryptor.encrypt(b"data", &[0u8; 32], &iv).is_ok());
}

#[test]
fn test_xchacha20_pack_open_roundtrip() {
    // 走库的公共 pack/open 路径：产物头部不存 nonce，
    // open 时按 iv_len()==24 现场派生，必须还原出原始字节。
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("source");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), "xchacha roundtrip payload").unwrap();
    std::fs::write(src.join("b.bin"), [0u8, 1, 2, 3, 254, 255]).unwrap();
    let out = temp.path().join("rt.secunzip");

    let config = PackConfig {
        format: OutputFormat::SecUnzip,
        compress: CompressAlgo::Zip,
        crypto: CryptoAlgo::XChaCha20,
        hash: HashAlgo::Sha256,
        key_derive: KeyNode::Input(KeySource::Literal("xchacha rt key".into())),
        run_mode: RunMode::TempDir,
        auth_mode: AuthMode::Local,
        expire_at: None,
        ip_whitelist: Vec::new(),
        salt: b"xchacha rt salt 32 bytes long!!!".to_vec(),
        app_id: None,
        allow_temp: false,
    };

    PackBuilder::new(config, vec![src], out.clone())
        .build()
        .unwrap();

    let loader = RuntimeLoader::from_file(&out).unwrap();
    let vfs = loader.mount_vfs_with_key("xchacha rt key").unwrap();
    assert_eq!(vfs.read_text("a.txt").unwrap(), "xchacha roundtrip payload");
    assert_eq!(
        vfs.read_file("b.bin").unwrap(),
        &[0u8, 1, 2, 3, 254, 255][..]
    );
}

#[test]
fn test_xchacha20_tampering_detected_by_integrity_hash() {
    // XChaCha20 是流密码，篡改密文本层不会报错，而是解出错误明文；
    // 检测由文件头的 SHA-256 integrity_hash 完成，这里验证端到端确实会被发现。
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("source");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), "xchacha payload").unwrap();
    let out = temp.path().join("t.secunzip");

    let config = PackConfig {
        format: OutputFormat::SecUnzip,
        compress: CompressAlgo::Zip,
        crypto: CryptoAlgo::XChaCha20,
        hash: HashAlgo::Sha256,
        key_derive: KeyNode::Input(KeySource::Literal("xchacha key".into())),
        run_mode: RunMode::TempDir,
        auth_mode: AuthMode::Local,
        expire_at: None,
        ip_whitelist: Vec::new(),
        salt: b"xchacha salt 32 bytes long!!!!!".to_vec(),
        app_id: None,
        allow_temp: false,
    };

    PackBuilder::new(config, vec![src], out.clone())
        .build()
        .unwrap();

    // 未篡改时能正常打开
    let loader = RuntimeLoader::from_file(&out).unwrap();
    let vfs = loader.mount_vfs_with_key("xchacha key").unwrap();
    assert_eq!(vfs.read_text("a.txt").unwrap(), "xchacha payload");

    let (offset, size) = {
        let loader = RuntimeLoader::from_file(&out).unwrap();
        let header = loader.header();
        (header.data_offset as usize, header.data_size as usize)
    };
    assert!(size > 0, "密文区不应为空");

    let mut data = std::fs::read(&out).unwrap();
    data[offset] ^= 0x01;

    let loader = RuntimeLoader::from_bytes(&data).unwrap();
    // VirtualFS 未实现 Debug，不能用 unwrap_err
    let err = match loader.mount_vfs_with_key("xchacha key") {
        Ok(_) => panic!("篡改密文必须被完整性校验发现"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("完整性校验"),
        "应由完整性校验发现篡改，实际: {}",
        err
    );
}

#[test]
fn test_sm4_cbc_returns_error_not_panic() {
    let encryptor = create_encryptor(&CryptoAlgo::Sm4Cbc);
    let key = vec![0x42u8; 32];
    let iv = vec![0x24u8; 16];

    let err = encryptor.encrypt(b"data", &key, &iv).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("SM4"), "错误信息必须点明算法: {}", msg);
    assert!(
        encryptor.decrypt(b"data", &key, &iv).is_err(),
        "SM4-CBC 解密同样必须返回错误而不是 panic"
    );
}

#[test]
fn test_sm3_returns_error_not_panic() {
    // Box<dyn Hasher> 无 Debug，不能用 unwrap_err
    let err = match create_hasher(&HashAlgo::Sm3) {
        Ok(_) => panic!("SM3 尚未实现，工厂不应返回哈希器"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(msg.contains("SM3"), "错误信息必须点明算法: {}", msg);
}

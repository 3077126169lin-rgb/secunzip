use secunzip::crypto::{create_encryptor, hash::create_hasher, keywrap::derive_key_iv};
use secunzip::core::{CryptoAlgo, HashAlgo};

#[test]
fn test_aes256_encrypt_decrypt() {
    let encryptor = create_encryptor(&CryptoAlgo::Aes256Cbc);
    
    let key = vec![0x42u8; 32]; // 32字节密钥
    let iv = vec![0x24u8; 16];  // 16字节IV
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
    let iv = vec![0x24u8; 12];  // 12字节nonce
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
    assert!(encryptor.decrypt(&ct, &key, &nonce).is_err(), "GCM 应检测出密文被篡改");

    // 篡改认证 tag 同样必须失败
    let mut ct2 = encryptor.encrypt(plaintext, &key, &nonce).unwrap();
    let last = ct2.len() - 1;
    ct2[last] ^= 0x01;
    assert!(encryptor.decrypt(&ct2, &key, &nonce).is_err(), "GCM 应检测出 tag 被篡改");
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
    let hasher = create_hasher(&HashAlgo::Sha256);
    let hash = hasher.hash(data);
    assert_eq!(hash.len(), 32);
    
    // SHA-512
    let hasher = create_hasher(&HashAlgo::Sha512);
    let hash = hasher.hash(data);
    assert_eq!(hash.len(), 64);
    
    // BLAKE3
    let hasher = create_hasher(&HashAlgo::Blake3);
    let hash = hasher.hash(data);
    assert_eq!(hash.len(), 32);
}

#[test]
fn test_hash_deterministic() {
    let data = b"consistent data";
    let hasher = create_hasher(&HashAlgo::Sha256);
    
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

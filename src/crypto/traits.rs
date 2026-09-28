use crate::core::CryptoAlgo;
use crate::crypto::aes::{Aes256CbcEncryptor, Aes256GcmEncryptor};
use crate::crypto::chacha::ChaCha20Encryptor;

/// 加密后端 trait - 可插拔设计
pub trait Encryptor: Send + Sync {
    /// 加密数据
    fn encrypt(&self, plaintext: &[u8], key: &[u8], iv: &[u8]) -> crate::Result<Vec<u8>>;

    /// 解密数据
    fn decrypt(&self, ciphertext: &[u8], key: &[u8], iv: &[u8]) -> crate::Result<Vec<u8>>;

    /// 密钥长度（字节）
    fn key_len(&self) -> usize;

    /// IV 长度（字节）
    fn iv_len(&self) -> usize;

    /// 算法名称
    fn algo_name(&self) -> &str;
}

/// 哈希后端 trait
pub trait Hasher: Send + Sync {
    /// 计算哈希
    fn hash(&self, data: &[u8]) -> Vec<u8>;

    /// 哈希输出长度（字节）
    fn output_len(&self) -> usize;

    /// 算法名称
    fn algo_name(&self) -> &str;
}

/// 根据算法枚举创建加密器
pub fn create_encryptor(algo: &CryptoAlgo) -> Box<dyn Encryptor> {
    match algo {
        CryptoAlgo::Aes256Cbc => Box::new(Aes256CbcEncryptor),
        CryptoAlgo::Aes256Gcm => Box::new(Aes256GcmEncryptor),
        CryptoAlgo::ChaCha20 => Box::new(ChaCha20Encryptor),
        // TODO: 实现 SM4
        _ => unimplemented!("加密算法 {:?} 尚未实现", algo),
    }
}

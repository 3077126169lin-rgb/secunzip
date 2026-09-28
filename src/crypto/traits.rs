use crate::core::CryptoAlgo;
use crate::crypto::aes::{Aes256CbcEncryptor, Aes256GcmEncryptor};
use crate::crypto::chacha::ChaCha20Encryptor;
use chacha20::cipher::{KeyIvInit, StreamCipher};

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

/// XChaCha20：ChaCha20 的扩展 nonce 变体，nonce 长度 24 字节。
///
/// 产物头部不保存 nonce/IV：打包与打开都由 `derive_key_iv` 按 `iv_len()`
/// 从密钥材料现场派生，这里 `iv_len()` 返回 24，与 XNonce 的原生长度一致。
///
/// nonce 映射（唯一确定，双向一致）：派生的 24 字节全部原样作为 `XNonce`，
/// 即 nonce = iv[0..24]。不补零、不截断。
/// 注意：`iv_len` 由 12 改为 24 会改变同一密钥材料派生出的密钥与 nonce
/// （`derive_key_iv` 的 PBKDF2 输出总长变了），因此新旧构造互不兼容；
/// 该改动仅因 XChaCha20 从未随任何版本发布、磁盘上不存在旧构造产物才安全。
///
/// XChaCha20 是流密码，密文不带认证标签：篡改密文不会被本层拒绝，
/// 而是解出错误明文，由文件头的 SHA-256 `integrity_hash` 校验兜底。
pub struct XChaCha20Encryptor;

impl Encryptor for XChaCha20Encryptor {
    fn encrypt(&self, plaintext: &[u8], key: &[u8], iv: &[u8]) -> crate::Result<Vec<u8>> {
        if key.len() != 32 {
            return Err(crate::SecUnzipError::Crypto(
                "XChaCha20 密钥必须 32 字节".into(),
            ));
        }
        if iv.len() != 24 {
            return Err(crate::SecUnzipError::Crypto(
                "XChaCha20 nonce 必须 24 字节".into(),
            ));
        }

        // 派生的 24 字节全部直接作为 XNonce，不补零、不截断
        let mut cipher = chacha20::XChaCha20::new(key.into(), chacha20::XNonce::from_slice(iv));
        let mut ciphertext = plaintext.to_vec();
        cipher.apply_keystream(&mut ciphertext);
        Ok(ciphertext)
    }

    fn decrypt(&self, ciphertext: &[u8], key: &[u8], iv: &[u8]) -> crate::Result<Vec<u8>> {
        // XChaCha20 是流密码，解密就是再次加密
        self.encrypt(ciphertext, key, iv)
    }

    fn key_len(&self) -> usize {
        32
    }

    /// XChaCha20 原生 nonce 长度：24 字节，派生值全部使用
    fn iv_len(&self) -> usize {
        24
    }

    fn algo_name(&self) -> &str {
        "XChaCha20"
    }
}

/// 尚未实现的加密算法占位实现。
///
/// `create_encryptor` 的签名不返回 Result（调用方在 builder/loader 中据此调用），
/// 因此工厂处无法直接给出错误；把错误推迟到真正调用 encrypt/decrypt 时抛出，
/// 替代原先的 `unimplemented!()` —— 用户看到「SM4-CBC 尚未实现」而不是进程 panic。
struct UnsupportedEncryptor(&'static str);

impl Encryptor for UnsupportedEncryptor {
    fn encrypt(&self, _plaintext: &[u8], _key: &[u8], _iv: &[u8]) -> crate::Result<Vec<u8>> {
        Err(crate::SecUnzipError::Crypto(format!("{} 尚未实现", self.0)))
    }

    fn decrypt(&self, _ciphertext: &[u8], _key: &[u8], _iv: &[u8]) -> crate::Result<Vec<u8>> {
        Err(crate::SecUnzipError::Decrypt(format!(
            "{} 尚未实现",
            self.0
        )))
    }

    /// 占位值：加密必然失败，此长度只用于 `derive_key_iv` 的入参，不影响任何产物
    fn key_len(&self) -> usize {
        32
    }

    /// 占位值：同上
    fn iv_len(&self) -> usize {
        12
    }

    fn algo_name(&self) -> &str {
        self.0
    }
}

/// 根据算法枚举创建加密器
///
/// 不写 `_ =>` 兜底：新增变体时必须在此显式处理，避免又退回静默/panic 分支。
pub fn create_encryptor(algo: &CryptoAlgo) -> Box<dyn Encryptor> {
    match algo {
        CryptoAlgo::Aes256Cbc => Box::new(Aes256CbcEncryptor),
        CryptoAlgo::Aes256Gcm => Box::new(Aes256GcmEncryptor),
        CryptoAlgo::ChaCha20 => Box::new(ChaCha20Encryptor),
        CryptoAlgo::XChaCha20 => Box::new(XChaCha20Encryptor),
        // TODO: 实现 SM4
        CryptoAlgo::Sm4Cbc => Box::new(UnsupportedEncryptor("SM4-CBC")),
    }
}

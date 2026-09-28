use aes::cipher::{block_padding::Pkcs7, KeyIvInit, BlockDecryptMut, BlockEncryptMut};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::Aes256Gcm;
use super::traits::Encryptor;

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

pub struct Aes256CbcEncryptor;

impl Encryptor for Aes256CbcEncryptor {
    fn encrypt(&self, plaintext: &[u8], key: &[u8], iv: &[u8]) -> crate::Result<Vec<u8>> {
        if key.len() != 32 {
            return Err(crate::SecUnzipError::Crypto("AES-256 密钥必须 32 字节".into()));
        }
        if iv.len() != 16 {
            return Err(crate::SecUnzipError::Crypto("AES CBC IV 必须 16 字节".into()));
        }

        let encryptor = Aes256CbcEnc::new(key.into(), iv.into());
        let mut buf = vec![0u8; plaintext.len() + 16]; // 预留padding空间
        let ct_len = encryptor
            .encrypt_padded_b2b_mut::<Pkcs7>(plaintext, &mut buf)
            .map_err(|e| crate::SecUnzipError::Crypto(format!("加密失败: {}", e)))?
            .len();
        buf.truncate(ct_len);
        Ok(buf)
    }

    fn decrypt(&self, ciphertext: &[u8], key: &[u8], iv: &[u8]) -> crate::Result<Vec<u8>> {
        if key.len() != 32 {
            return Err(crate::SecUnzipError::Decrypt("AES-256 密钥必须 32 字节".into()));
        }
        if iv.len() != 16 {
            return Err(crate::SecUnzipError::Decrypt("AES CBC IV 必须 16 字节".into()));
        }

        let decryptor = Aes256CbcDec::new(key.into(), iv.into());
        let mut buf = vec![0u8; ciphertext.len()];
        let pt_len = decryptor
            .decrypt_padded_b2b_mut::<Pkcs7>(ciphertext, &mut buf)
            .map_err(|e| crate::SecUnzipError::Decrypt(format!("解密失败: {}", e)))?
            .len();
        buf.truncate(pt_len);
        Ok(buf)
    }

    fn key_len(&self) -> usize {
        32
    }

    fn iv_len(&self) -> usize {
        16
    }

    fn algo_name(&self) -> &str {
        "AES-256-CBC"
    }
}

/// AES-256-GCM：认证加密（密文尾部带 16 字节 tag），可检测密文被篡改。
/// 注意：与 CBC 不同，GCM 的 nonce 长度是 12 字节。
pub struct Aes256GcmEncryptor;

impl Encryptor for Aes256GcmEncryptor {
    fn encrypt(&self, plaintext: &[u8], key: &[u8], iv: &[u8]) -> crate::Result<Vec<u8>> {
        if key.len() != 32 {
            return Err(crate::SecUnzipError::Crypto("AES-256-GCM 密钥必须 32 字节".into()));
        }
        if iv.len() != 12 {
            return Err(crate::SecUnzipError::Crypto("AES-GCM nonce 必须 12 字节".into()));
        }
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|_| crate::SecUnzipError::Crypto("GCM 密钥初始化失败".into()))?;
        cipher
            .encrypt(aes_gcm::Nonce::from_slice(iv), plaintext)
            .map_err(|_| crate::SecUnzipError::Crypto("GCM 加密失败".into()))
    }

    fn decrypt(&self, ciphertext: &[u8], key: &[u8], iv: &[u8]) -> crate::Result<Vec<u8>> {
        if key.len() != 32 {
            return Err(crate::SecUnzipError::Decrypt("AES-256-GCM 密钥必须 32 字节".into()));
        }
        if iv.len() != 12 {
            return Err(crate::SecUnzipError::Decrypt("AES-GCM nonce 必须 12 字节".into()));
        }
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|_| crate::SecUnzipError::Decrypt("GCM 密钥初始化失败".into()))?;
        cipher
            .decrypt(aes_gcm::Nonce::from_slice(iv), ciphertext)
            .map_err(|_| crate::SecUnzipError::Decrypt("解密失败：密钥错误或数据已被篡改".into()))
    }

    fn key_len(&self) -> usize {
        32
    }

    fn iv_len(&self) -> usize {
        12
    }

    fn algo_name(&self) -> &str {
        "AES-256-GCM"
    }
}

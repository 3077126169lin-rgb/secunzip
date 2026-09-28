use super::traits::Encryptor;
use chacha20::cipher::{KeyIvInit, StreamCipher};

pub struct ChaCha20Encryptor;

impl Encryptor for ChaCha20Encryptor {
    fn encrypt(&self, plaintext: &[u8], key: &[u8], iv: &[u8]) -> crate::Result<Vec<u8>> {
        if key.len() != 32 {
            return Err(crate::SecUnzipError::Crypto(
                "ChaCha20 密钥必须 32 字节".into(),
            ));
        }
        if iv.len() != 12 {
            return Err(crate::SecUnzipError::Crypto(
                "ChaCha20 nonce 必须 12 字节".into(),
            ));
        }

        let mut cipher = chacha20::ChaCha20::new(key.into(), iv.into());
        let mut ciphertext = plaintext.to_vec();
        cipher.apply_keystream(&mut ciphertext);
        Ok(ciphertext)
    }

    fn decrypt(&self, ciphertext: &[u8], key: &[u8], iv: &[u8]) -> crate::Result<Vec<u8>> {
        // ChaCha20 是流密码，解密就是再次加密
        self.encrypt(ciphertext, key, iv)
    }

    fn key_len(&self) -> usize {
        32
    }

    fn iv_len(&self) -> usize {
        12
    }

    fn algo_name(&self) -> &str {
        "ChaCha20"
    }
}

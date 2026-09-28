use pbkdf2::{pbkdf2_hmac};
use sha2::Sha256;

/// 从派生材料生成固定长度的密钥
/// 
/// 使用 PBKDF2 将任意长度的派生材料扩展为加密所需的密钥
pub fn derive_key_material(material: &[u8], salt: &[u8], key_len: usize) -> Vec<u8> {
    let mut key = vec![0u8; key_len];
    // 10000 轮迭代，平衡安全性和性能
    pbkdf2_hmac::<Sha256>(material, salt, 10_000, &mut key);
    key
}

/// 从派生材料生成加密密钥和IV
pub fn derive_key_iv(material: &[u8], salt: &[u8], key_len: usize, iv_len: usize) -> (Vec<u8>, Vec<u8>) {
    let total = key_len + iv_len;
    let derived = derive_key_material(material, salt, total);
    let key = derived[..key_len].to_vec();
    let iv = derived[key_len..].to_vec();
    (key, iv)
}

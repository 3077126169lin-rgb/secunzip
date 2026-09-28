use super::traits::Hasher;
use crate::core::HashAlgo;
use md5::Md5;
use sha2::{Digest, Sha256, Sha512};

pub struct Sha256Hasher;
pub struct Sha512Hasher;
pub struct Blake3Hasher;
pub struct Md5Hasher;

impl Hasher for Sha256Hasher {
    fn hash(&self, data: &[u8]) -> Vec<u8> {
        Sha256::digest(data).to_vec()
    }
    fn output_len(&self) -> usize {
        32
    }
    fn algo_name(&self) -> &str {
        "SHA-256"
    }
}

impl Hasher for Sha512Hasher {
    fn hash(&self, data: &[u8]) -> Vec<u8> {
        Sha512::digest(data).to_vec()
    }
    fn output_len(&self) -> usize {
        64
    }
    fn algo_name(&self) -> &str {
        "SHA-512"
    }
}

impl Hasher for Blake3Hasher {
    fn hash(&self, data: &[u8]) -> Vec<u8> {
        blake3::hash(data).as_bytes().to_vec()
    }
    fn output_len(&self) -> usize {
        32
    }
    fn algo_name(&self) -> &str {
        "BLAKE3"
    }
}

impl Hasher for Md5Hasher {
    fn hash(&self, data: &[u8]) -> Vec<u8> {
        use md5::Digest;
        Md5::digest(data).to_vec()
    }
    fn output_len(&self) -> usize {
        16
    }
    fn algo_name(&self) -> &str {
        "MD5"
    }
}

/// 根据算法枚举创建哈希器
///
/// 返回 Result 而不是 panic：`Hasher::hash` 的返回类型无法承载错误，
/// 若在工厂里 `unimplemented!()`，选择 SM3 的用户只会看到进程崩溃。
/// 不写 `_ =>` 兜底：新增变体时必须在此显式处理。
pub fn create_hasher(algo: &HashAlgo) -> crate::Result<Box<dyn Hasher>> {
    match algo {
        HashAlgo::Sha256 => Ok(Box::new(Sha256Hasher)),
        HashAlgo::Sha512 => Ok(Box::new(Sha512Hasher)),
        HashAlgo::Blake3 => Ok(Box::new(Blake3Hasher)),
        // TODO: 实现 SM3（国密哈希）
        HashAlgo::Sm3 => Err(crate::SecUnzipError::Crypto("SM3 尚未实现".into())),
    }
}

/// 计算 MD5 十六进制（小写）——用作文件 ID
pub fn md5_hex(data: &[u8]) -> String {
    use md5::Digest;
    Md5::digest(data)
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// 计算文件的 MD5 十六进制（小写）——文件 ID
pub fn file_md5_hex(path: &std::path::Path) -> crate::Result<String> {
    let data = std::fs::read(path)?;
    Ok(md5_hex(&data))
}

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SecUnzipError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("加密错误: {0}")]
    Crypto(String),

    #[error("解密错误: {0}")]
    Decrypt(String),

    #[error("密钥派生失败: {0}")]
    KeyDerivation(String),

    #[error("打包错误: {0}")]
    Packing(String),

    #[error("解包错误: {0}")]
    Unpacking(String),

    #[error("权限校验失败: {0}")]
    AuthFailed(String),

    #[error("内容大小超限: {size} bytes (最大 {max} bytes)")]
    SizeLimitExceeded { size: u64, max: u64 },

    #[error("格式错误: {0}")]
    Format(String),

    #[error("平台不支持: {0}")]
    PlatformNotSupported(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SecUnzipError>;

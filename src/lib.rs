pub mod cli;
pub mod core;
pub mod crypto;
pub mod key_derive;
pub mod packer;
pub mod runtime;

pub use core::{Result, SecUnzipError};

/// 最大支持文件大小：2GB
pub const MAX_CONTENT_SIZE: u64 = 2 * 1024 * 1024 * 1024;

/// 打包格式魔数
pub const MAGIC: &[u8; 8] = b"SECUNZIP";

/// 当前格式版本
pub const FORMAT_VERSION: u16 = 1;

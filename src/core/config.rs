use super::types::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 项目配置文件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    pub version: String,
    pub pack: PackConfig,
    /// 源文件/目录列表
    pub sources: Vec<PathBuf>,
    /// 输出路径
    pub output: PathBuf,
}

/// 打包产物头部结构
///
/// EXE格式：追加到exe尾部
/// 自有格式：文件头部
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackHeader {
    /// 魔数 "SECUNZIP"
    pub magic: [u8; 8],
    /// 格式版本
    pub version: u16,
    /// 输出格式
    pub format: OutputFormat,
    /// 打包配置
    pub config: PackConfig,
    /// 加密数据偏移量（相对文件开头）
    pub data_offset: u64,
    /// 加密数据大小
    pub data_size: u64,
    /// 原始数据大小（解压后）
    pub original_size: u64,
    /// 完整性校验（原始数据哈希）
    pub integrity_hash: Vec<u8>,
}

impl OutputFormat {
    fn to_u8(&self) -> u8 {
        match self {
            OutputFormat::Exe => 0,
            OutputFormat::SecUnzip => 1,
        }
    }
}

impl PackHeader {
    pub fn to_bytes(&self) -> crate::Result<Vec<u8>> {
        // 先序列化配置
        let config_bytes = bincode::serialize(&self.config)
            .map_err(|e| crate::SecUnzipError::Format(format!("序列化配置失败: {}", e)))?;

        // 构建完整头部
        let mut data = Vec::new();
        data.extend_from_slice(&self.magic);
        data.extend_from_slice(&self.version.to_le_bytes());
        data.extend_from_slice(&self.format.to_u8().to_le_bytes());
        data.extend_from_slice(&(config_bytes.len() as u32).to_le_bytes());
        data.extend_from_slice(&config_bytes);
        data.extend_from_slice(&self.data_offset.to_le_bytes());
        data.extend_from_slice(&self.data_size.to_le_bytes());
        data.extend_from_slice(&self.original_size.to_le_bytes());
        data.extend_from_slice(&(self.integrity_hash.len() as u32).to_le_bytes());
        data.extend_from_slice(&self.integrity_hash);

        Ok(data)
    }

    pub fn from_bytes(data: &[u8]) -> crate::Result<Self> {
        // 固定前缀：magic(8) + version(2) + format(1) + config_len(4)
        const PREFIX_LEN: usize = 15;
        // 配置之后的固定尾字段：data_offset(8) + data_size(8) + original_size(8) + hash_len(4)
        const TAIL_LEN: usize = 28;

        // 这是解析不可信输入的入口：每一步切片前都要先验证长度，
        // config_len 与 hash_len 都来自文件内容，必须用 checked_add 防溢出、再比长度。
        if data.len() < PREFIX_LEN {
            return Err(crate::SecUnzipError::Format("头部数据太短".into()));
        }

        let magic: [u8; 8] = data[0..8].try_into().unwrap();
        if &magic != crate::MAGIC {
            return Err(crate::SecUnzipError::Format("无效的魔数".into()));
        }

        let version = u16::from_le_bytes(data[8..10].try_into().unwrap());
        let format = match data[10] {
            0 => OutputFormat::Exe,
            1 => OutputFormat::SecUnzip,
            _ => return Err(crate::SecUnzipError::Format("无效的格式类型".into())),
        };

        let config_len = u32::from_le_bytes(data[11..15].try_into().unwrap()) as usize;
        let offset = PREFIX_LEN
            .checked_add(config_len)
            .ok_or_else(|| crate::SecUnzipError::Format("头部长度溢出".into()))?;
        let tail_start = offset
            .checked_add(TAIL_LEN)
            .ok_or_else(|| crate::SecUnzipError::Format("头部长度溢出".into()))?;
        if data.len() < tail_start {
            return Err(crate::SecUnzipError::Format("头部数据不完整".into()));
        }

        let config: PackConfig = bincode::deserialize(&data[PREFIX_LEN..offset])
            .map_err(|e| crate::SecUnzipError::Format(format!("反序列化配置失败: {}", e)))?;

        let data_offset = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        let data_size = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().unwrap());
        let original_size = u64::from_le_bytes(data[offset + 16..offset + 24].try_into().unwrap());

        let hash_len =
            u32::from_le_bytes(data[offset + 24..offset + 28].try_into().unwrap()) as usize;
        let hash_end = tail_start
            .checked_add(hash_len)
            .ok_or_else(|| crate::SecUnzipError::Format("完整性校验长度溢出".into()))?;
        if data.len() < hash_end {
            return Err(crate::SecUnzipError::Format("完整性校验字段不完整".into()));
        }
        let integrity_hash = data[tail_start..hash_end].to_vec();

        Ok(Self {
            magic,
            version,
            format,
            config,
            data_offset,
            data_size,
            original_size,
            integrity_hash,
        })
    }

    /// 计算头部总大小
    pub fn total_size(&self) -> usize {
        8 +  // magic
        2 +  // version
        1 +  // format
        4 +  // config_len
        bincode::serialize(&self.config).unwrap_or_default().len() +
        8 +  // data_offset
        8 +  // data_size
        8 +  // original_size
        4 +  // hash_len
        self.integrity_hash.len()
    }
}

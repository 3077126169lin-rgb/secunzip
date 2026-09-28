use super::compress::create_compressor;
use crate::core::{OutputFormat, PackConfig, PackHeader};
use crate::crypto::{create_encryptor, keywrap::derive_key_iv};
use crate::key_derive::KeyDeriveEngine;
use crate::{Result, FORMAT_VERSION, MAGIC, MAX_CONTENT_SIZE};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// 打包器
pub struct PackBuilder {
    config: PackConfig,
    sources: Vec<PathBuf>,
    output: PathBuf,
}

impl PackBuilder {
    pub fn new(config: PackConfig, sources: Vec<PathBuf>, output: PathBuf) -> Self {
        Self {
            config,
            sources,
            output,
        }
    }

    /// 执行打包并写入 self.output
    pub fn build(&self) -> Result<()> {
        let bytes = self.build_bytes()?;
        fs::write(&self.output, bytes)?;
        println!("[5/5] 完成: {}", self.output.display());
        Ok(())
    }

    /// 打包为字节（不落盘），便于"注册后再写文件"
    pub fn build_bytes(&self) -> Result<Vec<u8>> {
        // 1. 收集并压缩源文件
        println!("[1/5] 压缩源文件...");
        let compressed = self.compress_sources()?;

        // 2. 检查大小限制
        if compressed.len() as u64 > MAX_CONTENT_SIZE {
            return Err(crate::SecUnzipError::SizeLimitExceeded {
                size: compressed.len() as u64,
                max: MAX_CONTENT_SIZE,
            });
        }

        // 3. 派生密钥
        println!("[2/5] 派生密钥...");
        let engine = KeyDeriveEngine::new(self.config.salt.clone());
        let key_material = engine.derive(&self.config.key_derive)?;

        let encryptor = create_encryptor(&self.config.crypto);
        let (key, iv) = derive_key_iv(
            &key_material,
            &self.config.salt,
            encryptor.key_len(),
            encryptor.iv_len(),
        );

        // 4. 加密
        println!("[3/5] 加密数据...");
        let encrypted = encryptor.encrypt(&compressed, &key, &iv)?;

        // 5. 计算完整性哈希
        let integrity_hash = Sha256::digest(&compressed).to_vec();

        // 6. 根据格式组装
        println!("[4/5] 生成产物...");
        match self.config.format {
            OutputFormat::Exe => {
                self.build_exe(&encrypted, &integrity_hash, compressed.len() as u64)
            }
            OutputFormat::SecUnzip => {
                self.build_secunzip(&encrypted, &integrity_hash, compressed.len() as u64)
            }
        }
    }

    /// 压缩源文件
    fn compress_sources(&self) -> Result<Vec<u8>> {
        let compressor = create_compressor(&self.config.compress);
        // 如果只有一个目录，直接压缩目录
        if self.sources.len() == 1 && self.sources[0].is_dir() {
            return compressor.compress_dir(&self.sources[0]);
        }

        // 多个源文件/目录，创建临时目录收集
        let temp_dir = tempfile::tempdir()
            .map_err(|e| crate::SecUnzipError::Packing(format!("创建临时目录失败: {}", e)))?;

        for source in &self.sources {
            if source.is_dir() {
                copy_dir_recursive(source, &temp_dir.path().join(source.file_name().unwrap()))?;
            } else {
                let dest = temp_dir.path().join(source.file_name().unwrap());
                fs::copy(source, dest)?;
            }
        }

        compressor.compress_dir(temp_dir.path())
    }

    /// 写入文件头的配置
    ///
    /// 服务端托管模式（Remote）下，密钥只在服务端、不写入文件头，
    /// 避免把 content_key 泄漏在文件里（本地 extract 将不可用，必须联网取密钥）。
    fn header_config(&self) -> PackConfig {
        let mut c = self.config.clone();
        if matches!(self.config.auth_mode, crate::core::AuthMode::Remote(_)) {
            c.key_derive =
                crate::core::KeyNode::Input(crate::core::KeySource::Literal(String::new()));
        }
        c
    }

    /// 构建 EXE 格式
    ///
    /// 结构: [运行时代码] [头部] [加密数据]
    fn build_exe(
        &self,
        encrypted: &[u8],
        integrity_hash: &[u8],
        original_size: u64,
    ) -> Result<Vec<u8>> {
        // 1. 构造标准 .secunzip（[header][data]，自包含，data_offset 相对其起点）
        let mut embedded = self.build_secunzip(encrypted, integrity_hash, original_size)?;

        // 2. 存根 runner：优先用打包工具自身（双击可黑盒运行），否则退回占位模板
        let runner: Vec<u8> = std::env::current_exe()
            .ok()
            .and_then(|p| std::fs::read(p).ok())
            .unwrap_or_else(|| include_bytes!("../../assets/runtime_stub.exe").to_vec());

        // 3. 组装 [runner][.secunzip][trailer]，双击 runner 自查尾部标记进入黑盒模式
        let mut output = Vec::new();
        output.extend_from_slice(&runner);
        let embedded_offset = output.len() as u64;
        output.append(&mut embedded);
        // 尾部标记：[embedded_offset u64 LE][magic 8 字节]，固定 16 字节在末尾
        output.extend_from_slice(&embedded_offset.to_le_bytes());
        output.extend_from_slice(MAGIC);

        Ok(output)
    }

    /// 构建自有格式 .secunzip
    ///
    /// 结构: [文件头] [加密数据]
    fn build_secunzip(
        &self,
        encrypted: &[u8],
        integrity_hash: &[u8],
        original_size: u64,
    ) -> Result<Vec<u8>> {
        let header = PackHeader {
            magic: *MAGIC,
            version: FORMAT_VERSION,
            format: OutputFormat::SecUnzip,
            config: self.header_config(),
            data_offset: 0, // 稍后计算
            data_size: encrypted.len() as u64,
            original_size,
            integrity_hash: integrity_hash.to_vec(),
        };

        let header_bytes = header.to_bytes()?;
        let data_offset = header_bytes.len();

        let mut header = header;
        header.data_offset = data_offset as u64;
        let header_bytes = header.to_bytes()?;

        // 组装文件
        let mut output = Vec::new();
        output.extend_from_slice(&header_bytes);
        output.extend_from_slice(encrypted);

        Ok(output)
    }
}

/// 递归复制目录
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, dest_path)?;
        }
    }

    Ok(())
}

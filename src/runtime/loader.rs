use std::path::Path;
use sha2::{Sha256, Digest};
use crate::core::{PackHeader, OutputFormat};
use crate::crypto::{create_encryptor, keywrap::derive_key_iv};
use crate::key_derive::KeyDeriveEngine;
use crate::packer::compress::create_compressor;
use crate::Result;

/// 运行时加载器
pub struct RuntimeLoader {
    header: PackHeader,
    encrypted_data: Vec<u8>,
}

/// 解压结果
pub enum ExtractResult {
    /// 文件列表（文件名, 内容）
    Files(Vec<(String, Vec<u8>)>),
    /// 执行结果（仅EXE黑盒模式）
    Executed,
}

impl RuntimeLoader {
    /// 从文件加载
    pub fn from_file(path: &Path) -> Result<Self> {
        let data = std::fs::read(path)?;
        Self::from_bytes(&data)
    }

    /// 从字节加载
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        // 尝试解析头部
        let header = PackHeader::from_bytes(data)?;
        
        // 提取加密数据
        let data_start = header.data_offset as usize;
        let data_end = data_start + header.data_size as usize;
        
        if data_end > data.len() {
            return Err(crate::SecUnzipError::Format("数据不完整".into()));
        }
        
        let encrypted_data = data[data_start..data_end].to_vec();
        
        Ok(Self { header, encrypted_data })
    }

    /// 获取头部信息
    pub fn header(&self) -> &PackHeader {
        &self.header
    }

    /// 获取输出格式
    pub fn format(&self) -> &OutputFormat {
        &self.header.format
    }

    /// 执行解密和解压
    pub fn extract(&self) -> Result<ExtractResult> {
        // 1. 检查过期时间
        if let Some(expire) = &self.header.config.expire_at {
            let now = chrono::Local::now().format("%Y%m%d").to_string();
            if now > *expire {
                return Err(crate::SecUnzipError::AuthFailed("内容已过期".into()));
            }
        }

        // 2. 派生密钥
        let engine = KeyDeriveEngine::new(self.header.config.salt.clone());
        let key_material = engine.derive(&self.header.config.key_derive)?;
        
        let encryptor = create_encryptor(&self.header.config.crypto);
        let (key, iv) = derive_key_iv(
            &key_material,
            &self.header.config.salt,
            encryptor.key_len(),
            encryptor.iv_len(),
        );

        self.decrypt_and_decompress(&key, &iv)
    }

    /// 使用服务端下发的密钥字符串解密解压
    ///
    /// `key_str` 是打包时的 file_key（明文字符串）。
    /// 派生方式与 `extract()` 的 `KeyNode::Input(KeySource::Literal(key_str))` 完全一致，
    /// 保证服务端下发的密钥能解开打包侧加密的内容。
    pub fn extract_with_key(&self, key_str: &str) -> Result<ExtractResult> {
        // 1. 检查过期时间
        if let Some(expire) = &self.header.config.expire_at {
            let now = chrono::Local::now().format("%Y%m%d").to_string();
            if now > *expire {
                return Err(crate::SecUnzipError::AuthFailed("内容已过期".into()));
            }
        }

        // 2. 用服务端密钥派生（与打包侧 Literal(file_key) 一致）
        let node = crate::core::KeyNode::Input(crate::core::KeySource::Literal(key_str.to_string()));
        let engine = KeyDeriveEngine::new(self.header.config.salt.clone());
        let key_material = engine.derive(&node)?;

        let encryptor = create_encryptor(&self.header.config.crypto);
        let (key, iv) = derive_key_iv(
            &key_material,
            &self.header.config.salt,
            encryptor.key_len(),
            encryptor.iv_len(),
        );

        self.decrypt_and_decompress(&key, &iv)
    }

    /// 解密 + 校验 + 解压（共享逻辑）
    fn decrypt_and_decompress(&self, key: &[u8], iv: &[u8]) -> Result<ExtractResult> {
        // 3. 解密
        let encryptor = create_encryptor(&self.header.config.crypto);
        let compressed = encryptor.decrypt(&self.encrypted_data, key, iv)?;

        // 4. 验证完整性
        let hash = Sha256::digest(&compressed);
        if hash.as_slice() != self.header.integrity_hash.as_slice() {
            return Err(crate::SecUnzipError::Decrypt("完整性校验失败".into()));
        }

        // 5. 解压
        let compressor = create_compressor(&self.header.config.compress);
        let files = compressor.decompress(&compressed)?;

        // 6. 根据格式和模式处理
        match self.header.format {
            OutputFormat::Exe => {
                self.execute_sandbox(&files)?;
                Ok(ExtractResult::Executed)
            }
            OutputFormat::SecUnzip => {
                Ok(ExtractResult::Files(files))
            }
        }
    }

    /// 用服务端密钥解密解压到虚拟文件系统（内存，不落盘）
    pub fn mount_vfs_with_key(&self, key_str: &str) -> Result<crate::runtime::VirtualFS> {
        match self.extract_with_key(key_str)? {
            ExtractResult::Files(files) => Ok(crate::runtime::VirtualFS::from_files(files)),
            ExtractResult::Executed => Err(crate::SecUnzipError::Unpacking(
                "EXE 格式不支持文件浏览".into()
            )),
        }
    }

    /// 本地密钥派生后解密解压到虚拟文件系统（内存，不落盘）
    pub fn mount_vfs(&self) -> Result<crate::runtime::VirtualFS> {
        match self.extract()? {
            ExtractResult::Files(files) => Ok(crate::runtime::VirtualFS::from_files(files)),
            ExtractResult::Executed => Err(crate::SecUnzipError::Unpacking(
                "EXE 格式不支持文件浏览".into()
            )),
        }
    }

    /// 解压到指定目录
    pub fn extract_to_dir(&self, dest: &Path) -> Result<()> {
        // 1. 检查过期时间
        if let Some(expire) = &self.header.config.expire_at {
            let now = chrono::Local::now().format("%Y%m%d").to_string();
            if now > *expire {
                return Err(crate::SecUnzipError::AuthFailed("内容已过期".into()));
            }
        }

        // 2. 派生密钥
        let engine = KeyDeriveEngine::new(self.header.config.salt.clone());
        let key_material = engine.derive(&self.header.config.key_derive)?;
        
        let encryptor = create_encryptor(&self.header.config.crypto);
        let (key, iv) = derive_key_iv(
            &key_material,
            &self.header.config.salt,
            encryptor.key_len(),
            encryptor.iv_len(),
        );

        // 3. 解密
        let compressed = encryptor.decrypt(&self.encrypted_data, &key, &iv)?;

        // 4. 验证完整性
        let hash = Sha256::digest(&compressed);
        if hash.as_slice() != self.header.integrity_hash.as_slice() {
            return Err(crate::SecUnzipError::Decrypt("完整性校验失败".into()));
        }

        // 5. 解压到目录
        let compressor = create_compressor(&self.header.config.compress);
        let files = compressor.decompress(&compressed)?;
        
        std::fs::create_dir_all(dest)?;
        
        for (name, content) in files {
            let file_path = dest.join(&name);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(file_path, content)?;
        }

        Ok(())
    }

    /// 沙箱模式执行（内存）
    fn execute_sandbox(&self, files: &[(String, Vec<u8>)]) -> Result<()> {
        // TODO: 根据文件类型选择执行方式
        // - .exe: RunPE / 进程镂空
        // - .html/.js: 内嵌浏览器
        // - 其他: 临时落盘后用默认程序打开
        
        // 暂时简化：临时落盘执行
        let temp_dir = tempfile::tempdir()
            .map_err(|e| crate::SecUnzipError::Unpacking(format!("创建临时目录失败: {}", e)))?;
        
        for (name, content) in files {
            let file_path = temp_dir.path().join(name);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&file_path, content)?;
        }
        
        // 打开临时目录
        #[cfg(windows)]
        {
            std::process::Command::new("explorer.exe")
                .arg(temp_dir.path())
                .spawn()
                .map_err(|e| crate::SecUnzipError::Unpacking(format!("打开目录失败: {}", e)))?;
        }
        
        // 保持临时目录存在直到程序退出
        // TODO: 更优雅的处理
        std::thread::sleep(std::time::Duration::from_secs(1));
        
        Ok(())
    }
}

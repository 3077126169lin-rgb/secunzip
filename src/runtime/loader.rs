use crate::core::{OutputFormat, PackHeader};
use crate::crypto::{create_encryptor, keywrap::derive_key_iv};
use crate::key_derive::KeyDeriveEngine;
use crate::packer::compress::create_compressor;
use crate::Result;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// 校验并归一化 ZIP 条目名，返回可安全拼接到目标目录的相对路径。
///
/// 压缩层只回传字符串名（`ZipFile::enclosed_name()` 需要条目句柄，这里拿不到），
/// 因此按其语义重实现，并补上它不覆盖的两点：
/// - NUL 字节：`enclosed_name` 只返回 `None`，这里要给出可读原因；
/// - `\` 分隔符：Windows 上 `Path::components` 视其为分隔符，Linux 上却当成普通字符，
///   于是 `..\evil` 在 Linux 上能通过 `enclosed_name`。这里统一按两种分隔符拆段。
///
/// 拒绝以下形态（全部报中文错误并点名字条目，绝不静默跳过，否则数据会无声丢失）：
/// 空名、含 NUL 字节、绝对路径（`/x`、`\x`、`\\server\share`）、盘符前缀（`C:`、`c:/x`）、
/// 任何 `..` 段、归一化后为空的路径（如 `.`）。合法产物由 `add_dir_to_zip` 用真实相对
/// 路径生成，不会命中任何一条，因此不受影响。
fn safe_entry_path(name: &str) -> Result<PathBuf> {
    if name.is_empty() {
        return Err(crate::SecUnzipError::Unpacking(
            "ZIP 条目名为空，拒绝解压".into(),
        ));
    }
    if name.contains('\0') {
        return Err(crate::SecUnzipError::Unpacking(format!(
            "ZIP 条目名含 NUL 字节，拒绝解压: {:?}",
            name
        )));
    }
    // 以根或 UNC 开头：/etc/passwd、\Windows\...、\\server\share\...
    if name.starts_with('/') || name.starts_with('\\') {
        return Err(crate::SecUnzipError::Unpacking(format!(
            "ZIP 条目名为绝对路径，拒绝解压: {}",
            name
        )));
    }
    // 盘符前缀：C:\x、c:/x、C:..\..\x
    let raw = name.as_bytes();
    if raw.len() >= 2 && raw[1] == b':' && raw[0].is_ascii_alphabetic() {
        return Err(crate::SecUnzipError::Unpacking(format!(
            "ZIP 条目名含盘符前缀，拒绝解压: {}",
            name
        )));
    }

    // 两种分隔符都当分隔符，逐段判定后拼回相对路径
    let mut safe = PathBuf::new();
    for segment in name.split(['/', '\\']) {
        match segment {
            "" | "." => continue,
            ".." => {
                return Err(crate::SecUnzipError::Unpacking(format!(
                    "ZIP 条目名含上级目录片段 ..，拒绝解压: {}",
                    name
                )));
            }
            s => safe.push(s),
        }
    }
    if safe.as_os_str().is_empty() {
        return Err(crate::SecUnzipError::Unpacking(format!(
            "ZIP 条目名归一化后为空，拒绝解压: {}",
            name
        )));
    }
    Ok(safe)
}

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
        // data_offset/data_size 直接取自文件头，是不可信输入：先做 checked 转换与加法再比长度，
        // 与 config.rs::from_bytes 同一做法。若直接用 `+`，debug 下会因溢出 panic，release 下会
        // 回绕成 data_start > data_end，随后切片 panic。
        let data_start = usize::try_from(header.data_offset).map_err(|_| {
            crate::SecUnzipError::Format(format!(
                "加密数据偏移量 {} 超出本平台范围",
                header.data_offset
            ))
        })?;
        let data_size = usize::try_from(header.data_size).map_err(|_| {
            crate::SecUnzipError::Format(format!(
                "加密数据大小 {} 超出本平台范围",
                header.data_size
            ))
        })?;
        let data_end = data_start
            .checked_add(data_size)
            .ok_or_else(|| crate::SecUnzipError::Format("加密数据范围溢出".into()))?;

        if data_end > data.len() {
            return Err(crate::SecUnzipError::Format(format!(
                "数据不完整：加密数据范围 {}..{} 超出文件长度 {}",
                data_start,
                data_end,
                data.len()
            )));
        }

        let encrypted_data = data[data_start..data_end].to_vec();

        Ok(Self {
            header,
            encrypted_data,
        })
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
        let node =
            crate::core::KeyNode::Input(crate::core::KeySource::Literal(key_str.to_string()));
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

        // 6. 校验条目名（不可信输入）：这里是 extract/extract_with_key 的唯一出口，
        //    VFS 挂载、临时目录执行、CLI `open -o` 都从这里拿名字再 dest.join(name)，
        //    在出口挡住一次，等于所有落盘路径都不会写到目标目录之外。
        for (name, _) in &files {
            safe_entry_path(name)?;
        }

        // 7. 根据格式和模式处理
        match self.header.format {
            OutputFormat::Exe => {
                self.execute_sandbox(&files)?;
                Ok(ExtractResult::Executed)
            }
            OutputFormat::SecUnzip => Ok(ExtractResult::Files(files)),
        }
    }

    /// 用服务端密钥解密解压到虚拟文件系统（内存，不落盘）
    pub fn mount_vfs_with_key(&self, key_str: &str) -> Result<crate::runtime::VirtualFS> {
        match self.extract_with_key(key_str)? {
            ExtractResult::Files(files) => Ok(crate::runtime::VirtualFS::from_files(files)),
            ExtractResult::Executed => Err(crate::SecUnzipError::Unpacking(
                "EXE 格式不支持文件浏览".into(),
            )),
        }
    }

    /// 本地密钥派生后解密解压到虚拟文件系统（内存，不落盘）
    pub fn mount_vfs(&self) -> Result<crate::runtime::VirtualFS> {
        match self.extract()? {
            ExtractResult::Files(files) => Ok(crate::runtime::VirtualFS::from_files(files)),
            ExtractResult::Executed => Err(crate::SecUnzipError::Unpacking(
                "EXE 格式不支持文件浏览".into(),
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

        // 6. 先全量校验条目名，再创建目录落盘：否则前面的正常条目已经写进去，
        //    遇到恶意条目才报错，用户只会看到一个内容不全的目录。
        let mut pending = Vec::with_capacity(files.len());
        for (name, content) in files {
            pending.push((safe_entry_path(&name)?, content));
        }

        std::fs::create_dir_all(dest)?;

        for (relative, content) in pending {
            let file_path = dest.join(relative);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(file_path, content)?;
        }

        Ok(())
    }

    /// EXE 黑盒执行
    ///
    /// 当前实现是先把文件解到临时目录，再用资源管理器打开该目录，**不是内存执行**；
    /// 真正的内存执行在 feature 门控的 `runpe` 模块里，默认不编译。
    fn execute_sandbox(&self, files: &[(String, Vec<u8>)]) -> Result<()> {
        // TODO: 根据文件类型选择执行方式
        // - .exe: RunPE / 进程镂空
        // - .html/.js: 内嵌浏览器
        // - 其他: 临时落盘后用默认程序打开

        // 暂时简化：临时落盘执行
        let run_dir = prepare_run_dir()?;

        for (name, content) in files {
            let file_path = run_dir.join(name);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&file_path, content)?;
        }

        // 打开临时目录
        #[cfg(windows)]
        {
            std::process::Command::new("explorer.exe")
                .arg(&run_dir)
                .spawn()
                .map_err(|e| crate::SecUnzipError::Unpacking(format!("打开目录失败: {}", e)))?;
        }

        // 这里不再删除目录：此前用 TempDir 在函数返回时立即整棵删除，只 sleep 1 秒，
        // 与仍在启动/读文件的程序竞争，用户会看到文件缺失或程序半启动。
        // 现在目录保留在系统临时目录下，由下一次运行回收陈旧副本。
        Ok(())
    }
}

/// 陈旧运行目录的保留时长
const RUN_DIR_KEEP: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// 同一进程内的运行序号，保证同一时刻创建的目录名不冲突
static RUN_DIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 准备本次运行的临时目录。
///
/// 每次运行按 `pid + 时间戳 + 序号` 取唯一名字，进程退出后不主动删除，避免删掉
/// 仍在被程序使用的目录；下一次运行时只回收超过 `RUN_DIR_KEEP` 的旧目录，既不会
/// 误删正在运行的实例，也不会让临时目录无限堆积。
fn prepare_run_dir() -> Result<std::path::PathBuf> {
    let base = std::env::temp_dir().join("secunzip-runs");
    std::fs::create_dir_all(&base)
        .map_err(|e| crate::SecUnzipError::Unpacking(format!("创建临时目录失败: {}", e)))?;

    cleanup_stale_run_dirs(&base);

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = RUN_DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = base.join(format!("run-{}-{}-{}", std::process::id(), stamp, seq));
    std::fs::create_dir_all(&dir)
        .map_err(|e| crate::SecUnzipError::Unpacking(format!("创建临时目录失败: {}", e)))?;
    Ok(dir)
}

/// 回收陈旧目录；删除失败（仍被占用等）忽略，留待下次运行
fn cleanup_stale_run_dirs(base: &Path) {
    let entries = match std::fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|age| age > RUN_DIR_KEEP)
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{cleanup_stale_run_dirs, prepare_run_dir, safe_entry_path};
    use std::path::PathBuf;

    #[test]
    fn test_safe_entry_path_accepts_normal_names() {
        // 合法产物里的名字（含中文、子目录、`/` 与 `\` 混用）不能受影响
        for name in [
            "a.txt",
            "sub/nested.txt",
            "sub\\nested.txt",
            "./a.txt",
            "a//b.txt",
            "目录/文件.txt",
        ] {
            assert!(safe_entry_path(name).is_ok(), "合法条目名被拒绝: {}", name);
        }
        // 反斜杠按 Windows 语义当分隔符，而不是字面文件名
        assert_eq!(
            safe_entry_path("sub\\nested.txt").unwrap(),
            PathBuf::from("sub/nested.txt")
        );
    }

    #[test]
    fn test_safe_entry_path_rejects_every_escape_form() {
        let cases: &[(&str, &str)] = &[
            ("", "为空"),
            (".", "归一化后为空"),
            ("/etc/passwd", "绝对路径"),
            ("\\Windows\\System32\\evil.dll", "绝对路径"),
            ("\\\\server\\share\\evil.txt", "绝对路径"),
            ("C:\\Windows\\evil.dll", "盘符前缀"),
            ("c:/windows/evil.dll", "盘符前缀"),
            ("C:..\\..\\evil.txt", "盘符前缀"),
            ("../escape.txt", "上级目录"),
            ("..\\escape.txt", "上级目录"),
            ("..\\..\\Windows\\System32\\evil.dll", "上级目录"),
            ("sub/../../escape.txt", "上级目录"),
            ("sub/../escape.txt", "上级目录"),
            ("..", "上级目录"),
            ("a\0b.txt", "NUL"),
        ];
        for (name, expect) in cases {
            let err = safe_entry_path(name)
                .err()
                .unwrap_or_else(|| panic!("恶意条目名被接受: {:?}", name));
            let msg = err.to_string();
            assert!(
                msg.contains(expect),
                "错误信息应说明原因 {}，实际: {}",
                expect,
                msg
            );
            if !name.is_empty() {
                assert!(
                    msg.contains(name) || name.contains('\0'),
                    "错误信息应点名条目 {:?}，实际: {}",
                    name,
                    msg
                );
            }
        }
    }

    #[test]
    fn test_run_dir_is_unique_and_persistent() {
        let a = prepare_run_dir().unwrap();
        let b = prepare_run_dir().unwrap();
        assert_ne!(a, b, "两次运行必须拿到不同目录");
        assert!(a.is_dir() && b.is_dir());

        // 新鲜目录不能在一秒内被回收，否则又会和正在运行的程序竞争
        cleanup_stale_run_dirs(a.parent().unwrap());
        assert!(a.exists(), "刚刚创建的运行目录不应被回收");

        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn test_cleanup_missing_base_is_noop() {
        // base 不存在时不得 panic（例如首次运行）
        cleanup_stale_run_dirs(&std::env::temp_dir().join("secunzip-runs-not-exist"));
    }
}

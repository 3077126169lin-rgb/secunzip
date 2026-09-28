use crate::core::CompressAlgo;
use crate::Result;
use std::io::{Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;

/// 压缩后端 trait
pub trait Compressor: Send + Sync {
    /// 压缩目录内容
    fn compress_dir(&self, dir: &Path) -> Result<Vec<u8>>;

    /// 解压到内存
    fn decompress(&self, data: &[u8]) -> Result<Vec<(String, Vec<u8>)>>;

    /// 算法名称
    fn algo_name(&self) -> &str;
}

/// ZIP 压缩器
pub struct ZipCompressor;

impl Compressor for ZipCompressor {
    fn compress_dir(&self, dir: &Path) -> Result<Vec<u8>> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .compression_level(Some(6));

            add_dir_to_zip(&mut zip, dir, dir, options)?;
            zip.finish()
                .map_err(|e| crate::SecUnzipError::Packing(format!("ZIP完成失败: {}", e)))?;
        }
        Ok(buf.into_inner())
    }

    fn decompress(&self, data: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
        let reader = std::io::Cursor::new(data);
        let mut zip = zip::ZipArchive::new(reader)
            .map_err(|e| crate::SecUnzipError::Unpacking(format!("打开ZIP失败: {}", e)))?;

        let mut files = Vec::new();
        for i in 0..zip.len() {
            let mut file = zip
                .by_index(i)
                .map_err(|e| crate::SecUnzipError::Unpacking(format!("读取ZIP条目失败: {}", e)))?;

            if file.is_dir() {
                continue;
            }

            let name = file.name().to_string();
            let mut content = Vec::new();
            file.read_to_end(&mut content)
                .map_err(|e| crate::SecUnzipError::Unpacking(format!("读取文件内容失败: {}", e)))?;

            files.push((name, content));
        }
        Ok(files)
    }

    fn algo_name(&self) -> &str {
        "ZIP"
    }
}

/// 递归添加目录到ZIP
fn add_dir_to_zip<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    base: &Path,
    current: &Path,
    options: SimpleFileOptions,
) -> Result<()> {
    for entry in std::fs::read_dir(current)
        .map_err(|e| crate::SecUnzipError::Packing(format!("读取目录失败: {}", e)))?
    {
        let entry =
            entry.map_err(|e| crate::SecUnzipError::Packing(format!("读取目录项失败: {}", e)))?;
        let path = entry.path();

        let relative = path
            .strip_prefix(base)
            .map_err(|e| crate::SecUnzipError::Packing(format!("路径计算失败: {}", e)))?;
        let name = relative.to_string_lossy().replace('\\', "/");

        if path.is_dir() {
            zip.add_directory(&name, options)
                .map_err(|e| crate::SecUnzipError::Packing(format!("添加目录失败: {}", e)))?;
            add_dir_to_zip(zip, base, &path, options)?;
        } else {
            zip.start_file(&name, options)
                .map_err(|e| crate::SecUnzipError::Packing(format!("创建ZIP文件失败: {}", e)))?;
            let content = std::fs::read(&path).map_err(|e| {
                crate::SecUnzipError::Packing(format!(
                    "读取文件失败 {}: {}",
                    path.display(),
                    explain_io(&e)
                ))
            })?;
            zip.write_all(&content)
                .map_err(|e| crate::SecUnzipError::Packing(format!("写入ZIP失败: {}", e)))?;
        }
    }
    Ok(())
}

/// 解释 IO 错误，给出可读中文提示（如杀软拦截、拒绝访问）
fn explain_io(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(225) => {
            "操作被安全软件（如 Windows Defender/杀毒）拦截，请将本程序加入排除项后重试".to_string()
        }
        Some(5) => "拒绝访问（权限不足或文件被占用）".to_string(),
        _ => e.to_string(),
    }
}

/// 根据算法创建压缩器
///
/// 每个变体都显式列出，不写 `_ =>` 兜底：以前 SevenZ/TarZst/TarGz 会静默落进
/// ZIP 分支，用户以为拿到的是 7z，实际得到 ZIP —— 静默替换比报错更糟。
/// 未实现的算法返回 `UnsupportedCompressor`，在真正打包/解包时给出中文错误。
pub fn create_compressor(algo: &CompressAlgo) -> Box<dyn Compressor> {
    match algo {
        CompressAlgo::Store => Box::new(StoreCompressor),
        CompressAlgo::Zip => Box::new(ZipCompressor),
        CompressAlgo::SevenZ => Box::new(UnsupportedCompressor("7z")),
        CompressAlgo::TarZst => Box::new(UnsupportedCompressor("tar.zst")),
        CompressAlgo::TarGz => Box::new(UnsupportedCompressor("tar.gz")),
    }
}

/// 尚未实现的压缩算法占位实现。
///
/// `create_compressor` 的签名不返回 Result（调用方在 builder/loader 中据此调用），
/// 因此把错误推迟到真正调用 compress_dir/decompress 时抛出。
struct UnsupportedCompressor(&'static str);

impl Compressor for UnsupportedCompressor {
    fn compress_dir(&self, _dir: &Path) -> Result<Vec<u8>> {
        Err(crate::SecUnzipError::Packing(format!(
            "压缩算法 {} 尚未实现，请改用 Zip 或 Store",
            self.0
        )))
    }

    fn decompress(&self, _data: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
        Err(crate::SecUnzipError::Unpacking(format!(
            "压缩算法 {} 尚未实现",
            self.0
        )))
    }

    fn algo_name(&self) -> &str {
        self.0
    }
}

/// 无压缩，仅打包
struct StoreCompressor;

impl Compressor for StoreCompressor {
    fn compress_dir(&self, dir: &Path) -> Result<Vec<u8>> {
        // 使用ZIP但不压缩
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

            add_dir_to_zip(&mut zip, dir, dir, options)?;
            zip.finish()
                .map_err(|e| crate::SecUnzipError::Packing(format!("打包完成失败: {}", e)))?;
        }
        Ok(buf.into_inner())
    }

    fn decompress(&self, data: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
        ZipCompressor.decompress(data)
    }

    fn algo_name(&self) -> &str {
        "Store"
    }
}

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
pub fn create_compressor(algo: &CompressAlgo) -> Box<dyn Compressor> {
    match algo {
        CompressAlgo::Store => Box::new(StoreCompressor),
        // 其余统一用 ZIP（黑盒只支持一种打包格式）
        _ => Box::new(ZipCompressor),
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

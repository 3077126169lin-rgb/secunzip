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
        decompress_zip_capped(data, crate::MAX_CONTENT_SIZE)
    }

    fn algo_name(&self) -> &str {
        "ZIP"
    }
}

/// 按上限解压 ZIP 到内存。
///
/// 解压方向此前没有任何预算：单个条目可以把内存吃光，多条小条目累加同样没有上限
/// （ZIP 炸弹放大）。打包侧 `builder.rs` 已按 `MAX_CONTENT_SIZE` 限制，这里用同一
/// 常量把解压侧对齐，两侧不会漂移。`max` 参数化是为了让测试能用极小的上限验证
/// 逻辑，而不必真的造出 2 GiB 数据。
fn decompress_zip_capped(data: &[u8], max: u64) -> Result<Vec<(String, Vec<u8>)>> {
    let reader = std::io::Cursor::new(data);
    let mut zip = zip::ZipArchive::new(reader)
        .map_err(|e| crate::SecUnzipError::Unpacking(format!("打开ZIP失败: {}", e)))?;

    let mut files = Vec::new();
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| crate::SecUnzipError::Unpacking(format!("读取ZIP条目失败: {}", e)))?;

        if file.is_dir() {
            continue;
        }

        let name = file.name().to_string();

        // 先看元数据声明的解压后大小，超预算直接拒绝，不分配内存
        let declared = file.size();
        total = total.checked_add(declared).ok_or_else(|| {
            crate::SecUnzipError::Unpacking(format!(
                "解压后总大小超过上限 {}（条目 {}）",
                format_size(max),
                name
            ))
        })?;
        if total > max {
            return Err(crate::SecUnzipError::Unpacking(format!(
                "解压后总大小超过上限 {}（条目 {} 累计 {}）",
                format_size(max),
                name,
                format_size(total)
            )));
        }

        // 元数据可能被伪造，因此读取时再按剩余预算截断一次，双重保险
        let remaining = max - total + declared;
        let mut content = Vec::new();
        file.by_ref()
            .take(remaining.saturating_add(1))
            .read_to_end(&mut content)
            .map_err(|e| crate::SecUnzipError::Unpacking(format!("读取文件内容失败: {}", e)))?;

        if content.len() as u64 > remaining {
            return Err(crate::SecUnzipError::Unpacking(format!(
                "解压后总大小超过上限 {}（条目 {} 实际超出声明大小）",
                format_size(max),
                name
            )));
        }

        files.push((name, content));
    }
    Ok(files)
}

/// 把字节数格式化为可读文本（如 `2.00 GiB`、`512 B`）
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut idx = 0;
    while value >= 1024.0 && idx < UNITS.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.2} {}", value, UNITS[idx])
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个含若干条目的 ZIP：`entries` 为 (文件名, 解压后内容)
    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .compression_level(Some(9));
            for (name, content) in entries {
                zip.start_file(*name, options).unwrap();
                zip.write_all(content).unwrap();
            }
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn test_decompress_rejects_oversized_single_entry() {
        // 高度可压缩：1 MiB 的 0 压完只有几百字节，但解压后远超 1 KiB 上限
        let payload = vec![0u8; 1024 * 1024];
        let zip_data = make_zip(&[("bomb.bin", &payload)]);

        let err = decompress_zip_capped(&zip_data, 1024).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("超过上限"), "实际错误: {}", msg);
        assert!(msg.contains("bomb.bin"), "错误应点名条目: {}", msg);
    }

    #[test]
    fn test_decompress_rejects_running_total() {
        // 单条都不超限，但累加超过上限（多条小条目型放大）
        let chunk = vec![7u8; 800];
        let zip_data = make_zip(&[("a.bin", &chunk), ("b.bin", &chunk)]);

        // 单条 800 < 1000，两条 1600 > 1000
        let err = decompress_zip_capped(&zip_data, 1000).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("超过上限"), "实际错误: {}", msg);
        assert!(msg.contains("b.bin"), "错误应点名越界条目: {}", msg);
    }

    #[test]
    fn test_decompress_within_cap_roundtrips() {
        let zip_data = make_zip(&[("hello.txt", b"Hello, World!"), ("sub/n.bin", b"Nested")]);

        // 公共路径（真实 2 GiB 上限）必须行为不变
        let files = ZipCompressor.decompress(&zip_data).unwrap();
        assert_eq!(files.len(), 2);
        let hello = files.iter().find(|(n, _)| n == "hello.txt").unwrap();
        assert_eq!(hello.1, b"Hello, World!");

        // Store 走同一条 ZIP 路径，同样受上限保护
        let store = StoreCompressor.decompress(&zip_data).unwrap();
        assert_eq!(store.len(), 2);

        // 预算恰好等于解压后总大小：不越界，应成功
        let small_ok = decompress_zip_capped(&zip_data, 19).unwrap();
        assert_eq!(small_ok.len(), 2);
    }

    #[test]
    fn test_format_size_readable() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.00 KiB");
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.00 GiB");
    }
}

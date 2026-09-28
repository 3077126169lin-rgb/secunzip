//! 自解压黑盒 EXE 的检测与加载
//!
//! 产物布局: `[存根 runner][标准 .secunzip][尾部标记 16 字节]`
//! - 存根 runner：可运行程序（打包工具自身），双击后自查尾部标记进入黑盒模式；
//! - 标准 .secunzip：`[PackHeader][加密数据]`，data_offset 相对其起点，自包含；
//! - 尾部标记：`[embedded_offset u64 LE][magic 8 字节]`，固定在文件末尾。
//!
//! 这样既能双击黑盒运行，又能把内嵌部分当普通 .secunzip 解析，无需改动解压逻辑。

use crate::Result;

/// 尾部标记长度：embedded_offset(8) + magic(8)
pub const TRAILER_SIZE: usize = 16;

/// 尾部标记魔数（与 PackHeader 的 MAGIC 同为 "SECUNZIP"）
const TRAILER_MAGIC: &[u8] = b"SECUNZIP";

/// 检测是否为自解压黑盒 EXE；返回内嵌 .secunzip 的起始偏移
pub fn detect_self_extract(exe: &[u8]) -> Option<usize> {
    if exe.len() < TRAILER_SIZE {
        return None;
    }
    let trailer = &exe[exe.len() - TRAILER_SIZE..];
    if &trailer[8..16] != TRAILER_MAGIC {
        return None;
    }
    let embedded_offset = u64::from_le_bytes(trailer[0..8].try_into().ok()?) as usize;
    // 内嵌 .secunzip 必须落在存根之后、尾部标记之前
    if embedded_offset >= exe.len() - TRAILER_SIZE {
        return None;
    }
    Some(embedded_offset)
}

/// 取出内嵌的标准 .secunzip 字节（[PackHeader][加密数据]），可直接喂给 RuntimeLoader
pub fn embedded_secunzip(exe: &[u8]) -> Result<&[u8]> {
    let off = detect_self_extract(exe).ok_or_else(|| {
        crate::SecUnzipError::Unpacking("不是自解压黑盒 EXE（缺少尾部标记）".into())
    })?;
    Ok(&exe[off..exe.len() - TRAILER_SIZE])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_and_extract_embedded() {
        // 伪造 [runner][.secunzip][trailer]
        let runner = b"FAKE_RUNNER_BYTES".to_vec();
        let embedded = b"[HEADER][ENCRYPTED-DATA]".to_vec();
        let mut exe = runner.clone();
        let embedded_offset = exe.len() as u64;
        exe.extend_from_slice(&embedded);
        exe.extend_from_slice(&embedded_offset.to_le_bytes());
        exe.extend_from_slice(TRAILER_MAGIC);

        assert_eq!(detect_self_extract(&exe), Some(runner.len()));
        assert_eq!(embedded_secunzip(&exe).unwrap(), &embedded[..]);
    }

    #[test]
    fn reject_non_self_extract() {
        assert!(detect_self_extract(b"just a normal exe").is_none());
        assert!(detect_self_extract(b"").is_none());
    }
}

//! 解析健壮性：解析器面对不可信输入时只能返回错误，绝不能 panic。
//!
//! 这是 cargo-fuzz 的确定性退化版本：不依赖 nightly 与 libfuzzer，用固定的截断点、
//! 固定的位翻转位置和固定种子的伪随机数据覆盖解析路径，每次运行结果一致。

use secunzip::core::*;
use secunzip::packer::PackBuilder;
use secunzip::runtime::RuntimeLoader;
use std::panic::{self, AssertUnwindSafe};

fn sample_bytes() -> Vec<u8> {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(source_dir.join("a.txt"), "robustness sample content").unwrap();
    std::fs::write(source_dir.join("b.bin"), vec![0u8, 1, 2, 3, 255, 254, 253]).unwrap();

    let config = PackConfig {
        format: OutputFormat::SecUnzip,
        compress: CompressAlgo::Zip,
        crypto: CryptoAlgo::Aes256Gcm,
        hash: HashAlgo::Sha256,
        key_derive: KeyNode::Input(KeySource::Literal("robustness-key".into())),
        run_mode: RunMode::TempDir,
        auth_mode: AuthMode::Remote("http://127.0.0.1:1".into()),
        expire_at: None,
        ip_whitelist: Vec::new(),
        salt: b"robustness salt 32 bytes long!!!!".to_vec(),
        app_id: None,
        allow_temp: false,
    };
    let output = temp_dir.path().join("o.secunzip");
    PackBuilder::new(config, vec![source_dir], output.clone())
        .build()
        .unwrap();
    std::fs::read(&output).unwrap()
}

/// 返回 true 表示没有 panic
fn parses_without_panic(data: &[u8]) -> bool {
    panic::catch_unwind(AssertUnwindSafe(|| {
        let _ = RuntimeLoader::from_bytes(data);
    }))
    .is_ok()
}

/// 头部里 data_offset 字段的起始字节：固定前缀 magic(8)+version(2)+format(1)+config_len(4)=15，
/// 之后是长度可变的 config，再之后依次是 data_offset/data_size/original_size。
fn data_offset_field(bytes: &[u8]) -> usize {
    let config_len = u32::from_le_bytes(bytes[11..15].try_into().unwrap()) as usize;
    15 + config_len
}

/// 按小端把 u64 写回头部字段
fn patch_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn test_valid_sample_still_parses() {
    // 先确认样本本身可用，否则下面的用例会因为样本坏了而失去意义
    let bytes = sample_bytes();
    RuntimeLoader::from_bytes(&bytes).expect("正常产物应能解析");
}

#[test]
fn test_parser_never_panics_on_truncated_input() {
    let bytes = sample_bytes();
    let mut panicked = Vec::new();
    for cut in 0..=bytes.len() {
        if !parses_without_panic(&bytes[..cut]) {
            panicked.push(cut);
        }
    }
    assert!(
        panicked.is_empty(),
        "截断到这些长度时解析器 panic 了: {:?}",
        panicked
    );
}

#[test]
fn test_parser_never_panics_on_bit_flips() {
    let bytes = sample_bytes();
    let mut panicked = Vec::new();
    // 逐字节翻遍 8 个位太慢，固定每 64 字节取一个采样点，仍然覆盖头部与密文两段
    for offset in (0..bytes.len()).step_by(64) {
        for bit in 0..8u32 {
            let mut corrupted = bytes.clone();
            corrupted[offset] ^= 1u8 << bit;
            if !parses_without_panic(&corrupted) {
                panicked.push((offset, bit));
            }
        }
    }
    assert!(
        panicked.is_empty(),
        "翻转这些字节位时解析器 panic 了: {:?}",
        panicked
    );
}

#[test]
fn test_parser_rejects_out_of_range_data_fields() {
    // 逐字节采样翻位碰不到 data_offset/data_size 的高位字节，截断也造不出这些值，
    // 因此这里用合法产物改字节，直接构造「范围越界且 offset+size 溢出 u64」的组合。
    let bytes = sample_bytes();
    let field = data_offset_field(&bytes);
    let file_len = bytes.len() as u64;

    let cases: &[(&str, u64, u64)] = &[
        ("偏移 u64::MAX", u64::MAX, 0),
        ("偏移 u64::MAX-1，大小 1", u64::MAX - 1, 1),
        ("偏移 u64::MAX，大小 1（相加回绕到 0）", u64::MAX, 1),
        ("偏移 100，大小 u64::MAX（相加回绕到 99）", 100, u64::MAX),
        ("偏移刚好越过文件末尾", file_len + 1, 0),
        ("偏移越过文件末尾且相加溢出", file_len + 1, u64::MAX),
        ("大小 u64::MAX", 0, u64::MAX),
    ];

    for (name, offset, size) in cases {
        let mut corrupted = bytes.clone();
        patch_u64(&mut corrupted, field, *offset);
        patch_u64(&mut corrupted, field + 8, *size);

        // 旧实现在这里 debug 溢出 panic / release 回绕后切片 panic，两者都要挡住
        assert!(
            parses_without_panic(&corrupted),
            "{}：解析器 panic 了，应返回错误",
            name
        );
        let err = RuntimeLoader::from_bytes(&corrupted)
            .err()
            .unwrap_or_else(|| panic!("{}：越界的加密数据范围不应被接受", name));
        let msg = err.to_string();
        assert!(
            msg.contains("数据") || msg.contains("溢出") || msg.contains("范围"),
            "{}：错误信息应说明加密数据范围有问题，实际: {}",
            name,
            msg
        );
    }
}

#[test]
fn test_parser_rejects_garbage_without_panic() {
    let lengths = [0usize, 1, 7, 63, 64, 255, 1024, 4096];
    for len in lengths {
        // 固定种子的线性同余，避免引入随机数依赖
        let mut seed: u64 = 0x9E3779B97F4A7C15;
        let mut data = Vec::with_capacity(len);
        for _ in 0..len {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            data.push((seed >> 33) as u8);
        }
        assert!(
            parses_without_panic(&data),
            "长度为 {} 的随机数据让解析器 panic 了",
            len
        );
        assert!(
            RuntimeLoader::from_bytes(&data).is_err(),
            "长度为 {} 的随机数据不应被当成合法产物接受",
            len
        );
    }

    // 全 0 与全 0xFF 这两类边界填充
    for filler in [0u8, 0xFF] {
        for len in [1usize, 100, 4096] {
            let data = vec![filler; len];
            assert!(
                parses_without_panic(&data),
                "全 {:#04X} 的 {} 字节数据让解析器 panic 了",
                filler,
                len
            );
        }
    }
}

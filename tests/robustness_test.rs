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

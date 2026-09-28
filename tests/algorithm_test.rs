// 算法派发层的行为测试：未实现的压缩算法必须报错（不能静默替换成 ZIP），
// 已实现的 Zip/Store 必须保持原行为，未定义的密钥派生变换必须报错。
use secunzip::core::{
    AuthMode, CompressAlgo, CryptoAlgo, HashAlgo, KeyNode, KeySource, KeyTransform, OutputFormat,
    PackConfig, RunMode,
};
use secunzip::key_derive::engine::test_derivation;
use secunzip::packer::compress::create_compressor;
use secunzip::packer::PackBuilder;

#[test]
fn test_unimplemented_compress_algos_return_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "data").unwrap();

    let cases = [
        (CompressAlgo::SevenZ, "7z"),
        (CompressAlgo::TarZst, "tar.zst"),
        (CompressAlgo::TarGz, "tar.gz"),
    ];

    for (algo, name) in cases {
        let compressor = create_compressor(&algo);
        assert_eq!(compressor.algo_name(), name);

        let err = compressor.compress_dir(dir.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(name),
            "压缩错误必须点明算法 {}，实际: {}",
            name,
            msg
        );

        // 解压方向同样必须报错，不能静默走 ZIP
        assert!(
            compressor.decompress(b"whatever").is_err(),
            "{} 解压必须返回错误",
            name
        );
    }
}

#[test]
fn test_unimplemented_compress_algo_fails_pack() {
    // 端到端：选择 7z 打包必须失败，而不是产出一个 ZIP
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("source");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), "data").unwrap();
    let out = temp.path().join("t.secunzip");

    let config = PackConfig {
        format: OutputFormat::SecUnzip,
        compress: CompressAlgo::SevenZ,
        crypto: CryptoAlgo::Aes256Cbc,
        hash: HashAlgo::Sha256,
        key_derive: KeyNode::Input(KeySource::Literal("key".into())),
        run_mode: RunMode::TempDir,
        auth_mode: AuthMode::Local,
        expire_at: None,
        ip_whitelist: Vec::new(),
        salt: b"salt for sevenz test 32 bytes!!!".to_vec(),
        app_id: None,
        allow_temp: false,
    };

    let err = PackBuilder::new(config, vec![src], out.clone())
        .build()
        .unwrap_err();
    assert!(err.to_string().contains("7z"), "实际: {}", err);
    assert!(!out.exists(), "失败的打包不应留下产物");
}

#[test]
fn test_zip_and_store_still_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("hello.txt"), "Hello, World!").unwrap();
    std::fs::write(dir.path().join("sub").join("nested.txt"), "Nested").unwrap();

    for algo in [CompressAlgo::Zip, CompressAlgo::Store] {
        let compressor = create_compressor(&algo);
        let packed = compressor.compress_dir(dir.path()).unwrap();
        let files = compressor.decompress(&packed).unwrap();

        assert_eq!(files.len(), 2, "{:?} 应打包 2 个文件", algo);
        let mut names: Vec<String> = files.iter().map(|(n, _)| n.clone()).collect();
        names.sort();
        assert_eq!(
            names,
            vec!["hello.txt".to_string(), "sub/nested.txt".to_string()]
        );

        let hello = files.iter().find(|(n, _)| n == "hello.txt").unwrap();
        assert_eq!(String::from_utf8_lossy(&hello.1), "Hello, World!");
    }
}

#[test]
fn test_key_transform_concat_is_undefined() {
    // KeyTransform::Concat 没有可用的语义：以前它是 no-op（原样返回输入），
    // 现在必须报错，而不是假装变换成功。
    let node = KeyNode::Transform(
        KeyTransform::Concat,
        Box::new(KeyNode::Input(KeySource::Literal("abc".into()))),
    );

    let err = test_derivation(&node, b"salt").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("Concat"), "错误信息必须点明变换: {}", msg);

    // 真正的拼接走 KeyNode::Concat，仍然可用
    let concat = KeyNode::Concat(vec![
        KeyNode::Input(KeySource::Literal("abc".into())),
        KeyNode::Input(KeySource::Literal("def".into())),
    ]);
    assert!(test_derivation(&concat, b"salt").is_ok());
}

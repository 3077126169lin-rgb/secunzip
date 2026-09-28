use secunzip::core::{Condition, HashAlgo, KeyNode, KeySource, KeyTransform};
use secunzip::key_derive::KeyDeriveEngine;

#[test]
fn test_simple_input_derivation() {
    let engine = KeyDeriveEngine::new(b"test salt".to_vec());
    let node = KeyNode::Input(KeySource::Literal("my secret".into()));

    let key = engine.derive(&node).unwrap();
    assert_eq!(key.len(), 32); // 256 bits
}

#[test]
fn test_transform_hash() {
    let engine = KeyDeriveEngine::new(b"salt".to_vec());
    let node = KeyNode::Transform(
        KeyTransform::Hash(HashAlgo::Sha256),
        Box::new(KeyNode::Input(KeySource::Literal("data".into()))),
    );

    let key = engine.derive(&node).unwrap();
    assert_eq!(key.len(), 32);
}

#[test]
fn test_transform_take_first() {
    let engine = KeyDeriveEngine::new(b"salt".to_vec());
    let node = KeyNode::Transform(
        KeyTransform::TakeFirst(10),
        Box::new(KeyNode::Input(KeySource::Literal(
            "hello world this is a test".into(),
        ))),
    );

    let material = engine.derive(&node).unwrap();
    // 密钥派生后总是32字节
    assert_eq!(material.len(), 32);
}

#[test]
fn test_concat_nodes() {
    let engine = KeyDeriveEngine::new(b"salt".to_vec());
    let node = KeyNode::Concat(vec![
        KeyNode::Input(KeySource::Literal("part1_".into())),
        KeyNode::Input(KeySource::Literal("part2".into())),
    ]);

    let key = engine.derive(&node).unwrap();
    assert_eq!(key.len(), 32);
}

#[test]
fn test_conditional_true() {
    let engine = KeyDeriveEngine::new(b"salt".to_vec());
    let node = KeyNode::Conditional {
        condition: Condition::AlwaysTrue,
        then_branch: Box::new(KeyNode::Input(KeySource::Literal("yes".into()))),
        else_branch: Box::new(KeyNode::Input(KeySource::Literal("no".into()))),
    };

    let key = engine.derive(&node).unwrap();
    assert_eq!(key.len(), 32);
}

#[test]
fn test_conditional_false() {
    let engine = KeyDeriveEngine::new(b"salt".to_vec());
    let node = KeyNode::Conditional {
        condition: Condition::AlwaysFalse,
        then_branch: Box::new(KeyNode::Input(KeySource::Literal("yes".into()))),
        else_branch: Box::new(KeyNode::Input(KeySource::Literal("no".into()))),
    };

    let key = engine.derive(&node).unwrap();
    assert_eq!(key.len(), 32);
}

#[test]
fn test_same_input_same_key() {
    let salt = b"consistent salt";
    let engine1 = KeyDeriveEngine::new(salt.to_vec());
    let engine2 = KeyDeriveEngine::new(salt.to_vec());

    let node = KeyNode::Input(KeySource::Literal("deterministic".into()));

    let key1 = engine1.derive(&node).unwrap();
    let key2 = engine2.derive(&node).unwrap();

    assert_eq!(key1, key2);
}

#[test]
fn test_different_salt_different_key() {
    let engine1 = KeyDeriveEngine::new(b"salt1".to_vec());
    let engine2 = KeyDeriveEngine::new(b"salt2".to_vec());

    let node = KeyNode::Input(KeySource::Literal("same input".into()));

    let key1 = engine1.derive(&node).unwrap();
    let key2 = engine2.derive(&node).unwrap();

    assert_ne!(key1, key2);
}

#[test]
fn test_complex_derivation_flow() {
    // 模拟: MachineGUID -> 加盐 -> SHA256 -> 取前16字节
    let engine = KeyDeriveEngine::new(b"app salt".to_vec());
    let node = KeyNode::Transform(
        KeyTransform::TakeFirst(16),
        Box::new(KeyNode::Transform(
            KeyTransform::Hash(HashAlgo::Sha256),
            Box::new(KeyNode::Concat(vec![
                KeyNode::Input(KeySource::Literal("GUID-1234-5678".into())),
                KeyNode::Input(KeySource::Literal("20260921".into())),
            ])),
        )),
    );

    let key = engine.derive(&node).unwrap();
    assert_eq!(key.len(), 32); // PBKDF2 输出总是32字节
}

#[test]
fn test_date_condition() {
    let engine = KeyDeriveEngine::new(b"salt".to_vec());

    // 测试日期条件（总是能通过，因为用的是当前日期）
    let node = KeyNode::Conditional {
        condition: Condition::DateBefore("20991231".into()),
        then_branch: Box::new(KeyNode::Input(KeySource::Literal("valid".into()))),
        else_branch: Box::new(KeyNode::Input(KeySource::Literal("expired".into()))),
    };

    let key = engine.derive(&node).unwrap();
    assert_eq!(key.len(), 32);
}

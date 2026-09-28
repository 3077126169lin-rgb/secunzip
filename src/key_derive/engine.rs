use super::sources::{get_source_data, ip_in_cidr};
use crate::core::{Condition, KeyNode, KeyTransform};
use crate::crypto::hash::create_hasher;
use crate::Result;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

/// 密钥派生引擎
pub struct KeyDeriveEngine {
    /// 额外的盐值
    salt: Vec<u8>,
}

impl KeyDeriveEngine {
    pub fn new(salt: Vec<u8>) -> Self {
        Self { salt }
    }

    /// 执行密钥派生
    pub fn derive(&self, node: &KeyNode) -> Result<Vec<u8>> {
        let material = self.eval_node(node)?;

        // 使用 PBKDF2 将派生材料扩展为固定长度密钥
        Ok(crate::crypto::keywrap::derive_key_material(
            &material, &self.salt, 32, // 256 bits
        ))
    }

    /// 评估派生节点
    fn eval_node(&self, node: &KeyNode) -> Result<Vec<u8>> {
        match node {
            KeyNode::Input(source) => get_source_data(source),

            KeyNode::Transform(transform, inner) => {
                let data = self.eval_node(inner)?;
                self.apply_transform(transform, data)
            }

            KeyNode::Conditional {
                condition,
                then_branch,
                else_branch,
            } => {
                if self.eval_condition(condition)? {
                    self.eval_node(then_branch)
                } else {
                    self.eval_node(else_branch)
                }
            }

            KeyNode::Concat(nodes) => {
                let mut result = Vec::new();
                for node in nodes {
                    let data = self.eval_node(node)?;
                    result.extend_from_slice(&data);
                }
                Ok(result)
            }
        }
    }

    /// 应用变换
    fn apply_transform(&self, transform: &KeyTransform, data: Vec<u8>) -> Result<Vec<u8>> {
        match transform {
            KeyTransform::Hash(algo) => {
                let hasher = create_hasher(algo)?;
                Ok(hasher.hash(&data))
            }

            KeyTransform::TakeFirst(n) => {
                let n = (*n).min(data.len());
                Ok(data[..n].to_vec())
            }

            KeyTransform::TakeLast(n) => {
                let n = (*n).min(data.len());
                Ok(data[data.len() - n..].to_vec())
            }

            KeyTransform::AddSalt(salt) => {
                let mut result = data;
                result.extend_from_slice(salt);
                Ok(result)
            }

            // KeyTransform::Concat 没有语义：拼接多个节点由 KeyNode::Concat 承担，
            // 该变换自身只是「输入原样返回」，静默成功会让用户以为拼接已生效。
            KeyTransform::Concat => Err(crate::SecUnzipError::KeyDerivation(
                "密钥派生变换 Concat 未定义：拼接多个输入请使用 KeyNode::Concat 节点".into(),
            )),

            KeyTransform::Base64Encode => Ok(BASE64.encode(&data).into_bytes()),

            KeyTransform::Base64Decode => BASE64
                .decode(&data)
                .map_err(|e| crate::SecUnzipError::KeyDerivation(format!("Base64解码失败: {}", e))),
        }
    }

    /// 评估条件
    fn eval_condition(&self, condition: &Condition) -> Result<bool> {
        match condition {
            Condition::IpInRange(ip, cidr) => ip_in_cidr(ip, cidr),

            Condition::DateBefore(date_str) => {
                let current = chrono::Local::now().format("%Y%m%d").to_string();
                Ok(current <= *date_str)
            }

            Condition::DateAfter(date_str) => {
                let current = chrono::Local::now().format("%Y%m%d").to_string();
                Ok(current >= *date_str)
            }

            Condition::AlwaysTrue => Ok(true),
            Condition::AlwaysFalse => Ok(false),
        }
    }
}

/// 测试用的派生引擎
pub fn test_derivation(node: &KeyNode, salt: &[u8]) -> Result<Vec<u8>> {
    let engine = KeyDeriveEngine::new(salt.to_vec());
    engine.derive(node)
}

/// 由密钥生成流程（KeyNode 树）确定性生成 content_key（十六进制字符串）
/// 仅在打包时用于生成 content_key，随后托管到服务端；不写入文件头。
/// 空盐 ⇒ 完全由管理员定义的流程决定（可复现）。
pub fn generate_key_from_flow(node: &KeyNode) -> Result<String> {
    let engine = KeyDeriveEngine::new(Vec::new());
    let bytes = engine.derive(node)?;
    Ok(bytes.iter().map(|b| format!("{:02x}", b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{HashAlgo, KeyNode, KeySource, KeyTransform};

    #[test]
    fn flow_key_deterministic() {
        let node = KeyNode::Transform(
            KeyTransform::Hash(HashAlgo::Sha256),
            Box::new(KeyNode::Input(KeySource::Literal("hello".into()))),
        );
        let k1 = generate_key_from_flow(&node).unwrap();
        let k2 = generate_key_from_flow(&node).unwrap();
        assert_eq!(k1, k2, "同一流程应生成相同 content_key");
        assert_eq!(k1.len(), 64, "32 字节 → 64 个十六进制字符");
    }

    #[test]
    fn flow_key_differs_by_design() {
        let n1 = KeyNode::Input(KeySource::Literal("a".into()));
        let n2 = KeyNode::Input(KeySource::Literal("b".into()));
        assert_ne!(
            generate_key_from_flow(&n1).unwrap(),
            generate_key_from_flow(&n2).unwrap(),
            "不同流程应生成不同 content_key"
        );
    }
}

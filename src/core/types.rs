use serde::{Deserialize, Serialize};

/// 输出格式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutputFormat {
    /// 自解压 EXE，内存执行
    Exe,
    /// 自有格式，需要客户端打开
    SecUnzip,
}

/// 压缩算法
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompressAlgo {
    Zip,
    SevenZ,
    TarZst,
    TarGz,
    /// 无压缩，仅打包
    Store,
}

/// 加密算法
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CryptoAlgo {
    Aes256Cbc,
    Sm4Cbc,
    ChaCha20,
    XChaCha20,
    /// AES-256-GCM 认证加密（新文件默认）。
    /// 必须追加在枚举末尾：头部用 bincode 按「变体序号」编码，
    /// 插在中间会让旧文件里的 ChaCha20 被解析成 Sm4Cbc（破坏兼容）。
    Aes256Gcm,
}

/// 哈希算法
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HashAlgo {
    Sha256,
    Sha512,
    Sm3,
    Blake3,
}

/// 密钥派生来源
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeySource {
    /// Windows MachineGUID
    MachineGuid,
    /// 当前内网IP
    LocalIp,
    /// 当前日期 YYYYMMDD
    CurrentDate,
    /// 域名/用户名
    DomainUser,
    /// SteamID（预留）
    SteamId,
    /// 固定参数（打包时写入）
    Salt(Vec<u8>),
    /// 自定义字符串
    Literal(String),
}

/// 密钥派生变换
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyTransform {
    Hash(HashAlgo),
    /// 截取前N字节
    TakeFirst(usize),
    /// 截取后N字节
    TakeLast(usize),
    /// 加盐
    AddSalt(Vec<u8>),
    /// 拼接
    Concat,
    /// Base64编码
    Base64Encode,
    /// Base64解码
    Base64Decode,
}

/// 密钥派生节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyNode {
    /// 输入源
    Input(KeySource),
    /// 变换
    Transform(KeyTransform, Box<KeyNode>),
    /// 条件分支
    Conditional {
        condition: Condition,
        then_branch: Box<KeyNode>,
        else_branch: Box<KeyNode>,
    },
    /// 拼接多个节点
    Concat(Vec<KeyNode>),
}

/// 条件判断
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Condition {
    IpInRange(String, String),  // IP, CIDR
    DateBefore(String),         // YYYYMMDD
    DateAfter(String),
    AlwaysTrue,
    AlwaysFalse,
}

/// 运行模式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RunMode {
    /// 沙箱模式：内存解密执行（仅EXE格式）
    Sandbox,
    /// 文档模式：ISO挂载（仅EXE格式）
    Document,
    /// 落盘模式：解压到临时目录/用户选择目录
    TempDir,
}

/// 认证模式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthMode {
    /// 纯本地认证
    Local,
    /// 服务端认证
    Remote(String),  // 服务端URL
}

/// 打包配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackConfig {
    /// 输出格式
    pub format: OutputFormat,
    /// 压缩算法
    pub compress: CompressAlgo,
    /// 加密算法
    pub crypto: CryptoAlgo,
    /// 哈希算法
    pub hash: HashAlgo,
    /// 密钥派生规则
    pub key_derive: KeyNode,
    /// 运行模式
    pub run_mode: RunMode,
    /// 认证模式
    pub auth_mode: AuthMode,
    /// 过期时间 YYYYMMDD
    pub expire_at: Option<String>,
    /// IP白名单
    pub ip_whitelist: Vec<String>,
    /// 盐值
    pub salt: Vec<u8>,
    /// 应用ID（服务端模式）
    pub app_id: Option<String>,
    /// 允许临时权限申请
    pub allow_temp: bool,
}

impl Default for PackConfig {
    fn default() -> Self {
        Self {
            format: OutputFormat::Exe,
            compress: CompressAlgo::Zip,
            crypto: CryptoAlgo::Aes256Gcm,
            hash: HashAlgo::Sha256,
            key_derive: KeyNode::Input(KeySource::MachineGuid),
            run_mode: RunMode::Sandbox,
            auth_mode: AuthMode::Local,
            expire_at: None,
            ip_whitelist: Vec::new(),
            salt: rand::random::<[u8; 32]>().to_vec(),
            app_id: None,
            allow_temp: false,
        }
    }
}

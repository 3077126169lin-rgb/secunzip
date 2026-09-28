# SecUnZip 磁盘格式规范

适用范围：本仓库当前工作区（`Cargo.toml:10`，0.1.4）产出的 `.secunzip` 与黑盒自解压 EXE。
格式版本 `FORMAT_VERSION = 1`（`src/lib.rs:17`）。

本文件只描述磁盘字节：头部布局、加密层参数、归档层、完整性字段、黑盒容器、版本策略。
不描述服务端协议、授权流程与 GUI。每条结论都可以在标注的 `path:line` 处读到；无法从代码判定的条目一律标注「未确认」。

## 1. 概述

1. 产物有两种容器，共用同一套头部与同一套加密层。
   1. `.secunzip`：`[PackHeader][加密数据]`，由 `OutputFormat::SecUnzip` 分支产出（`src/packer/builder.rs:150-180`）。
   2. 黑盒自解压 EXE：`[runner][内嵌 .secunzip][尾部标记 16 字节]`，由 `OutputFormat::Exe` 分支产出（`src/packer/builder.rs:120-145`）。
2. 头部明文。加密数据的明文是 ZIP 归档，整体加密后紧随头部。
3. 头部总长度不固定：`config_len` 取决于 bincode 序列化 `PackConfig` 的实际字节数，随 `auth_mode` 中的 URL 长度、`key_derive` 树、`salt` 长度变化。
4. 字节序：头部中手工写入的整数一律小端（`src/core/config.rs:46-54`）；config 区由 bincode 生成，同为定长小端（§2.2）。
5. 读取入口是 `RuntimeLoader::from_bytes`（`src/runtime/loader.rs:94-112`），它先调 `PackHeader::from_bytes`，再按 `data_offset`/`data_size` 切出密文。

## 2. 头部逐字段布局

### 2.1 布局

记 `config_len` 为 config 区字节数，`hash_len` 为完整性哈希字节数。
固定前缀 15 字节、固定尾段 28 字节（`src/core/config.rs:59-63`），头部总长 = 15 + config_len + 28 + hash_len。

| 偏移 | 长度 | 字段 | 类型 | 说明 |
|------|------|------|------|------|
| 0 | 8 | magic | 原始 8 字节 | 必须等于 ASCII `SECUNZIP`（`src/lib.rs:14`，比较见 `src/core/config.rs:71-74`） |
| 8 | 2 | version | u16，小端 | 必须等于 `FORMAT_VERSION`，见 §7（`src/core/config.rs:76,82-95`） |
| 10 | 1 | format | u8 | 0 = Exe，1 = SecUnzip（`src/core/config.rs:28-35,97-101`）。该字节由手工映射写入，不来自 bincode |
| 11 | 4 | config_len | u32，小端 | config 区字节数 |
| 15 | config_len | config | bincode(`PackConfig`) | 见 §2.2 至 §2.5 |
| 15 + config_len | 8 | data_offset | u64，小端 | 密文相对**本容器起点**的偏移 |
| +8 | 8 | data_size | u64，小端 | 密文字节数 |
| +16 | 8 | original_size | u64，小端 | 写入的是 ZIP 归档的字节数，不是解压后文件总大小，且读路径不读它（§2.5） |
| +24 | 4 | hash_len | u32，小端 | `integrity_hash` 字节数 |
| +28 | hash_len | integrity_hash | 原始字节 | 见 §5 |

补充约束：

1. `data_offset` 由 builder 写两次（先写 0，算出头部长度后回填），最终等于头部长度（`src/packer/builder.rs:167-172`）。
2. 对黑盒 EXE，内嵌容器内的 `data_offset` 相对内嵌容器起点，不是相对 EXE 文件起点（`src/packer/builder.rs:126-127`，`src/runtime/selfextract.rs:3-8`）。
3. 解析不要求 `data_offset` 等于头部实际长度，也不要求头部结束到 `data_offset` 之间没有填充字节；读路径只按 `data_offset` 切片（`src/runtime/loader.rs:99-106`）。见 §9。

### 2.2 config 区的 bincode 编码

1. 写入用 `bincode::serialize`，读取用 `bincode::deserialize`（`src/core/config.rs:40,114`），依赖声明为 `bincode = "1.3"`（`Cargo.toml:38`）。
2. 顶层 `serialize`/`deserialize` 使用 fixint 编码、小端、允许尾随字节（bincode 1.3.3 `src/lib.rs:106-114,177-185`；默认为 `LittleEndian`，`src/config/mod.rs:100`）。兼容实现必须复现以下规则：
   1. 整数定长；长度前缀为 u64 小端（bincode 1.3.3 `src/config/int.rs:30-35`）。
   2. 枚举写 u32 小端变体序号（bincode 1.3.3 `src/ser/mod.rs:170-174`）。
   3. `Option` 为 1 字节标记（0 = None，1 = Some），Some 后接值（bincode 1.3.3 `src/ser/mod.rs:137-147`）。
   4. `bool` 为 1 字节（bincode 1.3.3 `src/ser/mod.rs:84-86`）。
   5. `String` 为 u64 字节长度 + UTF-8 字节（bincode 1.3.3 `src/ser/mod.rs:121-124`）。
   6. `Vec<T>` 为 u64 元素个数 + 各元素；`Vec<u8>` 走同一条序列路径（bincode 1.3.3 `src/ser/mod.rs:149-153`）。
   7. 结构体按字段声明顺序拼接，不带长度前缀；`usize` 由 serde 交给 `serialize_u64`，定长 8 字节。
3. config 区长度由外层 `config_len` 给出，bincode 自身不写长度；`config_len` 之后到切片末尾的多余字节被静默忽略（尾随字节规则）。

### 2.3 PackConfig 字段顺序

config 区就是 `PackConfig`（`src/core/types.rs:136-162`）的序列化，顺序固定，不可省略任何字段：

| 序号 | 字段 | 类型 | 取值说明 |
|------|------|------|----------|
| 1 | format | `OutputFormat` | 与头部第 10 字节独立，可以不一致（§2.5） |
| 2 | compress | `CompressAlgo` | 见 §4 |
| 3 | crypto | `CryptoAlgo` | 见 §3 |
| 4 | hash | `HashAlgo` | 只写不读（§2.5） |
| 5 | key_derive | `KeyNode` | 递归结构，见 §2.4、§3.3 |
| 6 | run_mode | `RunMode` | 只写不读（§2.5） |
| 7 | auth_mode | `AuthMode` | `Remote(url)` 时携带服务端地址 |
| 8 | expire_at | `Option<String>` | `YYYYMMDD`，读路径参与判断（§2.5） |
| 9 | ip_whitelist | `Vec<String>` | 只写不读（§2.5） |
| 10 | salt | `Vec<u8>` | 长度可变；生产写入 32 字节随机值（`src/cli/commands.rs:103`） |
| 11 | app_id | `Option<String>` | 打包路径恒为 `None`（`src/cli/commands.rs:104`） |
| 12 | allow_temp | `bool` | 只写不读（§2.5） |

### 2.4 枚举变体序号

序号即枚举中的声明位置，从 0 开始。这些序号直接进入磁盘字节，新增变体只能追加在末尾（`src/core/types.rs:30-33`）。

| 枚举 | 序号 = 名字 |
|------|-------------|
| `OutputFormat` | 0 = Exe，1 = SecUnzip（`src/core/types.rs:5-10`） |
| `CompressAlgo` | 0 = Zip，1 = SevenZ，2 = TarZst，3 = TarGz，4 = Store（`src/core/types.rs:14-21`） |
| `CryptoAlgo` | 0 = Aes256Cbc，1 = Sm4Cbc，2 = ChaCha20，3 = XChaCha20，4 = Aes256Gcm（`src/core/types.rs:25-34`） |
| `HashAlgo` | 0 = Sha256，1 = Sha512，2 = Sm3，3 = Blake3（`src/core/types.rs:38-43`） |
| `KeySource` | 0 = MachineGuid，1 = LocalIp，2 = CurrentDate，3 = DomainUser，4 = SteamId，5 = Salt(Vec\<u8\>)，6 = Literal(String)（`src/core/types.rs:47-62`） |
| `KeyTransform` | 0 = Hash(HashAlgo)，1 = TakeFirst(usize)，2 = TakeLast(usize)，3 = AddSalt(Vec\<u8\>)，4 = Concat，5 = Base64Encode，6 = Base64Decode（`src/core/types.rs:66-80`） |
| `KeyNode` | 0 = Input(KeySource)，1 = Transform(KeyTransform, Box\<KeyNode\>)，2 = Conditional{condition, then_branch, else_branch}，3 = Concat(Vec\<KeyNode\>)（`src/core/types.rs:84-97`） |
| `Condition` | 0 = IpInRange(String, String)，1 = DateBefore(String)，2 = DateAfter(String)，3 = AlwaysTrue，4 = AlwaysFalse（`src/core/types.rs:101-107`） |
| `RunMode` | 0 = Sandbox，1 = Document，2 = TempDir（`src/core/types.rs:117-124`） |
| `AuthMode` | 0 = Local，1 = Remote(String)（`src/core/types.rs:128-133`） |

`KeyNode` 的 1、2、3 号变体含子节点（2 号含 then/else 两个分支），递归编码；`Box<KeyNode>` 按 `KeyNode` 本身序列化。

长度推导（按上表逐步相加，未编译验证）：当 `auth_mode` 为 `Remote` 且 `key_derive` 为 `Literal` 时，
`config_len = 99 + L + N`，其中 `L` 是 `Literal` 字符串字节数、`N` 是服务端 URL 字节数；头部总长 `= 174 + L + N`。
`src/packer/builder.rs:108-115` 在 Remote 模式下把 `key_derive` 替换为 `Literal("")`，故生产路径 `L = 0`。

### 2.5 字段语义与只写不读的字段

1. 被读路径使用的字段：`magic`、`version`、头部的 `format` 字节、`data_offset`、`data_size`、`integrity_hash`（`src/core/config.rs:59-141`），以及 `config` 中的 `crypto`、`compress`、`key_derive`、`salt`、`auth_mode`、`expire_at`（`src/runtime/loader.rs:127-144,183-193`，`src/cli/commands.rs:229-234,1170-1173`）。
2. `original_size` 只写不读。builder 写入 `compressed.len()`，即 ZIP 归档字节数（`src/packer/builder.rs:66-76,124-127`），全仓库没有读取点。
3. `PackConfig` 中以下字段被写入但没有任何读取点。兼容实现必须仍按 §2.3 的类型与顺序序列化它们，即使它们不产生任何效果：
   1. `run_mode`：类型注释明确说明当前不读取、不分支（`src/core/types.rs:109-124`）。
   2. `app_id`：打包路径恒为 `None`（`src/cli/commands.rs:104`）。
   3. `hash`：完整性校验恒用 SHA-256，不读取该字段（`src/packer/builder.rs:6,66`）。
   4. `allow_temp`：是否开放临时申请由打包函数入参决定并只发给服务端（`src/cli/commands.rs:46,57,105`），头部副本不参与判断。
   5. `ip_whitelist`：头部副本不参与任何校验。
4. `expire_at` 会被读取：`extract`、`extract_with_key`、`extract_to_dir` 在派生密钥前比较有效期，规则为 `chrono::Local::now().format("%Y%m%d") > expire_at` 的字符串比较，命中即报「内容已过期」（`src/runtime/loader.rs:127-132,156-161,236-241`）。生产打包恒写 `None`（`src/cli/commands.rs:101`），该检查在实际产物上不会触发。
5. 头部 `format` 字节与 `config.format` 是两个独立字段，取值可以不一致。打包器产出的产物中头部字节恒为 1，因为 `build_secunzip` 硬编码 `OutputFormat::SecUnzip`（`src/packer/builder.rs:159`），黑盒 EXE 的内嵌容器也走这条路径；而 `config.format` 保留调用方设置的值（黑盒 EXE 为 `Exe`）。读路径按头部字节分支（`src/runtime/loader.rs:204-210`），因此 `OutputFormat::Exe` 分支对打包器产物不可达。
6. Remote 模式下头部 `key_derive` 被清空为 `Literal("")`（`src/packer/builder.rs:108-115`），头部不承载真实 `content_key`。实际密钥由服务端下发，读路径按 `Literal(key_str)` 重建派生树（`src/runtime/loader.rs:154-178`）。

## 3. 加密层

### 3.1 算法标识

1. 标识来自 `PackConfig.crypto` 的 bincode 变体序号（§2.4）。读路径按该值选实现（`src/runtime/loader.rs:138,183`，工厂见 `src/crypto/traits.rs:127-135`）。
2. 工厂不写兜底分支，新增变体必须在 `create_encryptor` 中显式处理（`src/crypto/traits.rs:126`）。

### 3.2 密钥长度、IV/nonce 长度与实现状态

| 算法（序号） | key_len | iv_len | 状态 |
|--------------|---------|--------|------|
| Aes256Cbc（0） | 32 | 16 | 已实现，PKCS7 填充（`src/crypto/aes.rs:11-71`） |
| Sm4Cbc（1） | 32（占位） | 12（占位） | 未实现：加解密直接返回错误（`src/crypto/traits.rs:95-122,134`） |
| ChaCha20（2） | 32 | 12 | 已实现，流密码（`src/crypto/chacha.rs:6-41`） |
| XChaCha20（3） | 32 | 24 | 已实现，24 字节派生值全部作为 `XNonce`，不补零、不截断（`src/crypto/traits.rs:51-88`，尤其 `63-66,80-83`） |
| Aes256Gcm（4） | 32 | 12 | 已实现，认证加密，密文尾部带 16 字节 tag（`src/crypto/aes.rs:73-124`，tag 长度见 `aes.rs:73` 注释） |

Sm4Cbc 的两个占位长度不产生任何字节：`UnsupportedEncryptor` 的 `encrypt`/`decrypt` 在触碰密钥长度前就返回错误（`src/crypto/traits.rs:98-107`），占位值只用于 `derive_key_iv` 的入参。因此磁盘上不会有 Sm4Cbc 产物，兼容实现只需能识别该序号并报错。

### 3.3 密钥与 IV 的派生

IV 不落盘。密钥与 IV 都由密钥材料现场派生（`src/crypto/keywrap.rs:15-25`）。派生分两级，两级都用 PBKDF2-HMAC-SHA256、10000 轮：

1. 第一级：`key_material = PBKDF2-HMAC-SHA256(节点求值结果, salt = config.salt, 输出 32 字节)`（`src/key_derive/engine.rs:19-26`，`src/crypto/keywrap.rs:7-12`）。
2. 第二级：`derived = PBKDF2-HMAC-SHA256(key_material, salt = config.salt, 输出 key_len + iv_len)`；
   `key = derived[0..key_len]`，`iv = derived[key_len..]`（`src/crypto/keywrap.rs:15-25`）。
3. `config.salt` 在两级各用一次。打包与打开调用同样的参数（`src/packer/builder.rs:50-59`，`src/runtime/loader.rs:135-144,166-175,244-253`）。
4. `key_len` 与 `iv_len` 因此属于格式契约：它们决定第二级 PBKDF2 的输出总长，改动会改变同一密钥材料派生出的 key 与 iv，新旧构造互不兼容（`src/crypto/traits.rs:43-45` 对此有说明）。

第一级的输入是 `config.key_derive` 这棵 `KeyNode` 树的求值结果。`KeySource::Literal(s)` 取 `s` 的 UTF-8 字节（`src/key_derive/sources.rs:13`）；其余来源（MachineGuid、LocalIp、CurrentDate、DomainUser、Salt、SteamId）只在打包侧或由打包侧构造的流程中使用，打开已发布产物时只用到 `Literal`（`src/runtime/loader.rs:164-165`）。`KeySource::SteamId` 返回「尚未实现」（`src/key_derive/sources.rs:14-19`）。

### 3.4 认证与篡改

1. XChaCha20 与 ChaCha20 是流密码，密文不带认证标签；篡改只会解出错误明文，由 §5 的哈希兜底（`src/crypto/traits.rs:47-48`，`src/crypto/chacha.rs:25-27`）。
2. Aes256Gcm 的解密失败与标签校验失败返回同一句「解密失败：密钥错误或数据已被篡改」（`src/crypto/aes.rs:107-111`）。
3. Aes256Cbc 的填充错误与完整性失败被故意合并为同一文案「完整性校验失败」，以消除 padding oracle 判别信号（`src/crypto/aes.rs:48-55`）。
4. 未确认：仓库内没有本格式各算法的已知答案测试向量（KAT）。`docs/bugs.md:349` 提到的那组 PBKDF2 向量是服务端口令散列的，与本格式的 KDF 无关。

## 4. 归档层

### 4.1 格式与算法取值

1. 归档格式是 ZIP（`zip = "2.2"`，`Cargo.toml:33`）。加密层的明文就是这个 ZIP 字节流。
2. `CompressAlgo::Zip` 写 Deflated，压缩级别 6（`src/packer/compress.rs:23-36`）。
3. `CompressAlgo::Store` 也写 ZIP 容器，但用 `CompressionMethod::Stored`；解压复用 ZIP 路径（`src/packer/compress.rs:218-244`）。
4. `CompressAlgo::SevenZ`、`TarZst`、`TarGz` 未实现：打包与解包都返回中文错误，不回退为 ZIP（`src/packer/compress.rs:182-216`）。
5. 结论：序号 0 与 4 的产物磁盘形态都是 ZIP，序号 1、2、3 不会出现在磁盘上。

### 4.2 条目名写入规则

1. 以源目录为基准取相对路径，`\` 替换为 `/`，随后写入 ZIP（`src/packer/compress.rs:140-143`）。
2. 目录条目用 `add_directory` 写入（`src/packer/compress.rs:145-148`）。
3. 多个源文件或目录时，先复制进临时目录再整目录压缩，因此条目名以各源的文件名/目录名为顶层（`src/packer/builder.rs:81-102`）。

### 4.3 条目名校验

所有条目名在落盘或返回给调用方之前都要过 `safe_entry_path`：`extract` 与 `extract_with_key` 共用 `decrypt_and_decompress`，在返回结果前逐个校验（`src/runtime/loader.rs:196-201`）；`extract_to_dir` 不调用该函数，而是在创建目标目录之前把全部条目先校验进 `pending`（`src/runtime/loader.rs:256-273`）。拒绝以下全部形态，报中文错误并点名字条目：

1. 空名（`src/runtime/loader.rs:22-26`）。
2. 含 NUL 字节（`src/runtime/loader.rs:27-32`）。
3. 以 `/` 或 `\` 开头，覆盖绝对路径与 `\\server\share` 形式的 UNC（`src/runtime/loader.rs:34-39`）。
4. 盘符前缀：第 2 个字节是 `:` 且第 1 个字节是 ASCII 字母，如 `C:\x`、`c:/x`、`C:..\..\x`（`src/runtime/loader.rs:41-47`）。
5. 任何一段恰好等于 `..`（`src/runtime/loader.rs:51-59`）。
6. 归一化后为空，如 `.`（`src/runtime/loader.rs:63-68`）。

被接受并跳过的段：空段与 `.`（`src/runtime/loader.rs:52-53`），因此 `./a.txt`、`a//b.txt` 合法。`/` 与 `\` 都按分隔符处理（`src/runtime/loader.rs:51`）。相应测试见 `src/runtime/loader.rs:379-438` 与 `tests/zip_slip_test.rs`。

### 4.4 目录条目

解压时目录条目被丢弃：`file.is_dir()` 直接跳过（`src/packer/compress.rs:65-67`）。解压结果只包含文件，空目录不会出现，也无法还原。

### 4.5 解压大小预算

1. 单次解压的「解压后总字节数」上限为 `MAX_CONTENT_SIZE` = 2 GiB = 2147483648（`src/lib.rs:11`，`src/packer/compress.rs:39`）。
2. 判定先用 `checked_add` 累加 ZIP 元数据声明的 `file.size()`，超限即拒绝，不分配内存；读取时再按剩余预算截断一次，防止声明值被伪造（`src/packer/compress.rs:71-103`）。
3. 打包侧的同一常量约束的是 ZIP 归档自身的字节数（`src/packer/builder.rs:41-46`），与解压侧口径不同：一个限归档大小，一个限解压后总大小。

## 5. 完整性

1. 算法恒为 SHA-256，输出 32 字节，不随 `PackConfig.hash` 变化（`src/packer/builder.rs:6,66`）。
2. 覆盖范围：加密层的明文，即 ZIP 归档的完整字节流（`src/packer/builder.rs:66` 对 `compressed` 求哈希）。不覆盖头部，不覆盖密文。
3. 存储位置：头部明文字段 `integrity_hash`（`src/core/config.rs:25,53-54`）。
4. 这是**无密钥哈希，不是 MAC**。头部明文，任何人可以改写哈希或 `config`；它能检测损坏与部分密文篡改，不提供来源认证。这与 `docs/technical.md` 的「完整性」一行、`docs/design.md` 的「安全边界」一节表述一致。
   （跨文档引用只写章节名不写行号：本仓库的行号引用会随编辑快速失效，已经因此产生过一批死引用。）
5. 读取顺序：先解密，再对解密结果求 SHA-256 并与头部字段逐字节比较，不等即报「完整性校验失败」（`src/runtime/loader.rs:184-190,256-262`）。两侧按 `as_slice()` 比较，长度不同也判失败。
6. `hash_len` 不校验具体取值，`from_bytes` 接受 0 到任意长度（`src/core/config.rs:121-129`）；读侧恒算 32 字节，因此非 32 字节的哈希必然不匹配。

## 6. 黑盒 EXE 容器

1. 布局（`src/packer/builder.rs:120-145`，`src/runtime/selfextract.rs:1-8`）：
   1. runner：打包进程自身的二进制，即打包时运行的 CLI 或 GUI 可执行文件（`src/packer/builder.rs:130-133`）。取不到自身路径时回退到编译期嵌入的 `assets/runtime_stub.exe`，其内容是 11 字节 ASCII `PLACEHOLDER`，不是可运行的 PE（`assets/README.md:5-12`）。
   2. 内嵌 `.secunzip`：一个完整、自包含的标准容器，其 `data_offset` 相对它自己的起点（`src/packer/builder.rs:126-127,168`）。
   3. 尾部标记：16 字节，`embedded_offset` u64 小端 + ASCII `SECUNZIP`（`src/packer/builder.rs:140-142`，`src/runtime/selfextract.rs:12-16`）。`embedded_offset` 是内嵌容器相对 EXE 文件起点的偏移，等于 runner 的字节数。
2. 读取规则：文件长度不小于 16、末尾 8 字节等于 `SECUNZIP`、且 `embedded_offset < 文件长度 - 16`，返回偏移（`src/runtime/selfextract.rs:19-33`）；`embedded_secunzip` 返回 `exe[offset .. len-16]`（`src/runtime/selfextract.rs:36-41`）。
3. 读取规则不校验 runner 区，也不校验内嵌头部的魔数；后者由 `PackHeader::from_bytes` 校验。
4. 自识别：CLI 在没有已知子命令时先只读末尾 16 字节做预筛，命中后整读自身并进入黑盒流程（`src/main.rs:13-26,91-110`）；GUI 启动时同样自查（`gui/src/app.rs:163-171`）。
5. 内嵌容器与独立 `.secunzip` 的差异只有两处：
   1. `config.format`：黑盒为 `Exe`（序号 0），独立容器为 `SecUnzip`（序号 1）。
   2. 头部第 10 字节 `format`：两者都是 1，因为 `build_secunzip` 硬编码 `OutputFormat::SecUnzip`（`src/packer/builder.rs:159`）。
   其余头部字段、加密层、归档层完全一致，读路径也一致（黑盒把内嵌切片交给 `RuntimeLoader::from_bytes`，`src/cli/commands.rs:198-199`）。
6. 文件 ID 口径：黑盒用 EXE 文件整体的 MD5，独立容器用 `.secunzip` 整体的 MD5；MD5 不写入文件头（`src/cli/commands.rs:52,200`，`src/crypto/hash.rs:76-82`）。
7. 未确认：在 runner 之后追加数据是否在所有目标 Windows 版本上都不影响 PE 加载与运行，仓库内没有自动化验证。代码依赖「追加后双击仍能运行」与「进程能整读自身」这两个行为（`src/packer/builder.rs:135-142`，`src/main.rs:99-109`）。

## 7. 版本

1. 当前 `FORMAT_VERSION = 1`（`src/lib.rs:17`），builder 写入该常量（`src/packer/builder.rs:158`）。
2. 解析时 `version` 在反序列化 config **之前**比较（`src/core/config.rs:76-95`）。高于与低于都被拒绝：未来版本可能改过 config 的编码方式，用今天的规则硬解会得到「看似成功、内容错乱」的结果；更小的值不存在合法产物，只可能是伪造或损坏（`src/core/config.rs:78-81`）。
3. 两条错误信息分别写明「高于」「低于」与两个版本号（`src/core/config.rs:82-95`），测试见 `tests/header_version_test.rs:56-111`。
4. 因为 bincode 按变体序号索引，新增算法或模式只能追加到枚举末尾（`src/core/types.rs:30-33`，`docs/technical.md:29`）。
5. 加未来版本的做法：
   1. 只在确实要改变头部布局、字段语义或编码方式时递增 `FORMAT_VERSION`，并同时保留对旧版本字节的解析分支。
   2. 仅追加枚举变体时不需要递增 `FORMAT_VERSION`：旧文件的序号不受影响，新文件里的新序号会让旧读者在 config 反序列化处失败。该失败的具体错误文案未在测试中固定（见 §9）。

## 8. 兼容性

### 8.1 必须对齐的点

1. 魔数、头部字段的偏移与长度、全部整数的字节序（§2.1）。
2. config 区必须按 bincode 1.3 的 fixint/小端规则编解码，且 `PackConfig` 字段顺序与 §2.4 的枚举序号完全一致。不能因为某些字段「读不到」就省略它们，省略会让后续所有字段错位。
3. 两级 PBKDF2 的参数（HMAC-SHA256、10000 轮、同一个 `salt` 用两次）与 `key || iv` 的切分方式（§3.3）。
4. 各算法的 `key_len`/`iv_len`（§3.2）。IV 必须由密钥材料派生，不得改为从文件读取或写回文件。
5. 完整性哈希是 SHA-256，只覆盖归档明文，不是 MAC（§5）。
6. 归档层必须是 ZIP，条目名规则与解压预算按 §4.3、§4.5 实现。
7. 黑盒容器的尾部 16 字节布局与 `embedded_offset` 的基准（§6.1）。
8. `version` 必须与自身支持的版本完全相等才接受（§7.2）。

### 8.2 已发布版本之间的差异

按仓库标签对比（`git diff v0.1.3`），`src/packer/builder.rs`、`src/lib.rs`、`src/core/types.rs` 自 v0.1.3 起未改动，即 MAGIC、`MAX_CONTENT_SIZE`、`FORMAT_VERSION`、头部布局、`PackConfig` 定义与写盘路径均未变。v0.1.0 至 v0.1.3 的 `FORMAT_VERSION` 同样是 1。

自 v0.1.3 起与格式相关的改动只有两处：

1. `from_bytes` 新增版本相等校验（`src/core/config.rs:78-95`）。此前不校验版本，因此旧程序读新文件不会报版本错误。
2. XChaCha20 的 `iv_len` 由 12 改为 24（`src/crypto/traits.rs:80-83` 与标签 v0.1.3 中同文件的 12 对照）。旧构造把 12 字节派生值放前、后 12 字节补零；新构造使用全部 24 字节。两者派生的 key 与 nonce 都不同，互不兼容。

第 2 条的适用范围可以收敛：v0.1.3 与当前的打包入口都固定写 `crypto: Aes256Gcm`、`compress: CompressAlgo::Zip`（`src/cli/commands.rs:95-96` 与标签 v0.1.3 中同一函数），GUI 也走同一个 `pack_file`（`gui/src/app.rs:550-557`），因此由发布版工具生成的产物只有 Aes256Gcm + Zip 一种组合，磁盘上不存在 12 字节 nonce 构造的 XChaCha20 产物。`src/crypto/traits.rs:44-45` 关于该改动安全的判断与此一致。

### 8.3 建议的兼容性测试

把某个已发布产物的完整字节固化进测试套件作为 fixture，用它断言：头部各字段解析结果、归档条目与内容、归档明文的 SHA-256 与冻结值一致。

当前仓库没有任何此类测试：`tests/` 下所有用例都是现场打包再读取，检索 `fixture`、`golden`、`frozen` 无命中；`tests/header_version_test.rs:14-39` 与 `tests/robustness_test.rs:11-37` 也在运行时构造样本。没有 fixture 时，枚举序号错位、PBKDF2 参数变化、字段顺序调整这类破坏兼容的改动不会被任何测试发现。

## 9. 已知缺口

以下条目是读者不能假定成立的地方。

1. 未确认：仓库内不存在本格式的已知答案测试向量，KDF 与加密参数只由本仓库代码自洽验证（§3.4）。
2. 未确认：仓库内不存在已发布产物的冻结字节样本，因此本文件描述的是当前代码的格式，无法逐字节证明与历史发布产物一致（§8.3）。
3. 未确认：在 runner 之后追加数据对 PE 加载的影响未经仓库内验证（§6.7）。
4. 未确认：旧读者遇到未来新增的枚举序号时，具体错误文案未在测试中固定（§7.5）。
5. `data_offset` 与 `data_size` 直接来自头部。读路径**此前**用未检查的加法计算密文区间，溢出组合（例如 `data_offset = 100`、`data_size = u64::MAX`）在 debug 下 panic、在 release 下回绕后得到起点大于终点的切片并 panic。现已改为 `usize::try_from` + `checked_add` + 与文件长度比对，非法组合返回「加密数据范围溢出」等错误；回归测试是 `tests/robustness_test.rs` 的 `test_parser_rejects_out_of_range_data_fields`，它覆盖 `u64::MAX` 与各种回绕组合（回退该修复后此测试在 debug 与 release 下都会失败，这是它有效的证据）。
6. 头部结束到 `data_offset` 之间可以有任意字节，`data_offset` 也可以指向头部内部；读路径不做这项一致性校验（§2.1）。
7. `PackHeader::total_size`（`src/core/config.rs:144-155`）在全仓库没有调用点，不要把它当作权威的头部长度来源。
8. `original_size` 无语义（§2.5）。
9. 条目名在两条落盘路径上的用法不同：`extract_to_dir` 使用 `safe_entry_path` 归一化后的路径（`src/runtime/loader.rs:270-283`），CLI 的 `open -o` 使用 ZIP 中的原始条目名 `dest.join(name)`（`src/cli/commands.rs:271-276`）。两条路径都先经过同一道校验，都不会写出目标目录之外；但含 `\` 的条目名在 CLI 落盘路径上依赖操作系统的分隔符语义。

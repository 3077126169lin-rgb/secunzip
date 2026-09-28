# SecUnzip 归属与第三方声明
# Attribution & Third-Party Notices

本文件声明 SecUnzip 所基于的技术、格式与开源组件，履行相应的归属与许可义务。

---

## 1. SecUnzip 基于的技术与格式

SecUnzip 的受控内容封装格式基于以下公开技术构建（自有格式，扩展名 `.secunzip`）：

| 方面 | 基于 | 说明 |
|------|------|------|
| **容器/归档** | **ZIP 归档格式** | 打包多个带路径的文件为单一归档；由 `zip` 库实现。扩展名改为自有 `.secunzip`。 |
| **对称加密** | **AES-256-GCM**（默认）/ **AES-256-CBC** / **ChaCha20** | 对归档整体加密；由 RustCrypto 项目实现。GCM 提供认证加密（密文带认证标签）。 |
| **密钥派生** | **PBKDF2-HMAC-SHA256**（10000 轮） | 由派生材料导出加密密钥与 IV；由 RustCrypto 项目实现。 |
| **完整性校验** | **SHA-256** | 归档明文的哈希，用于检测数据损坏；密文的真实性由 AES-256-GCM 认证标签保证。 |
| **哈希/文件 ID** | **SHA-256 / SHA-512 / BLAKE3 / MD5** | 文件 ID 取打包文件的 MD5（内容寻址）。 |
| **密钥托管** | 服务端下发 | 解密密钥仅托管于服务端，联网验证授权后下发，不写入文件。 |

> 说明：`.secunzip` 为**整体加密容器**（先归档、后整块加密），底层归档格式（ZIP）封装于密文之内，
> 对外不可直接识别；文件扩展名 `.secunzip` 为本项目自有。

---

## 2. 第三方开源组件（Third-Party Components）

SecUnzip 使用了以下开源组件。许可信息取自各 crate 自身的 `Cargo.toml`，点组件名可到
[crates.io](https://crates.io) 查阅完整许可文本。绝大多数采用 **MIT** 与 **Apache-2.0** 双许可（任选其一）。

### 密码学 / 核心库
| 组件 | 许可 |
|------|------|
| [`aes`](https://crates.io/crates/aes) | MIT OR Apache-2.0 |
| [`aes-gcm`](https://crates.io/crates/aes-gcm) | Apache-2.0 OR MIT |
| [`cbc`](https://crates.io/crates/cbc) | MIT OR Apache-2.0 |
| [`chacha20`](https://crates.io/crates/chacha20) | Apache-2.0 OR MIT |
| [`sha2`](https://crates.io/crates/sha2) | MIT OR Apache-2.0 |
| [`md-5`](https://crates.io/crates/md-5) | MIT OR Apache-2.0 |
| [`pbkdf2`](https://crates.io/crates/pbkdf2) | MIT OR Apache-2.0 |
| [`hkdf`](https://crates.io/crates/hkdf) | MIT OR Apache-2.0 |
| [`blake3`](https://crates.io/crates/blake3) | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |

### 归档 / 压缩 / 编码 / 序列化
| 组件 | 许可 |
|------|------|
| [`zip`](https://crates.io/crates/zip) | MIT |
| [`flate2`](https://crates.io/crates/flate2) | MIT OR Apache-2.0 |
| [`base64`](https://crates.io/crates/base64) | MIT OR Apache-2.0 |
| [`bincode`](https://crates.io/crates/bincode) | MIT |
| [`serde`](https://crates.io/crates/serde) | MIT OR Apache-2.0 |
| [`serde_json`](https://crates.io/crates/serde_json) | MIT OR Apache-2.0 |
| [`toml`](https://crates.io/crates/toml) | MIT OR Apache-2.0 |

### 运行时 / 网络 / 工具
| 组件 | 许可 |
|------|------|
| [`tokio`](https://crates.io/crates/tokio) | MIT |
| [`reqwest`](https://crates.io/crates/reqwest) | MIT OR Apache-2.0 |
| [`uuid`](https://crates.io/crates/uuid) | Apache-2.0 OR MIT |
| [`rand`](https://crates.io/crates/rand) | MIT OR Apache-2.0 |
| [`chrono`](https://crates.io/crates/chrono) | MIT OR Apache-2.0 |
| [`thiserror`](https://crates.io/crates/thiserror) | MIT OR Apache-2.0 |
| [`anyhow`](https://crates.io/crates/anyhow) | MIT OR Apache-2.0 |
| [`tracing`](https://crates.io/crates/tracing) | MIT |
| [`tracing-subscriber`](https://crates.io/crates/tracing-subscriber) | MIT |
| [`tempfile`](https://crates.io/crates/tempfile) | MIT OR Apache-2.0 |
| [`hostname`](https://crates.io/crates/hostname) | MIT |
| [`winreg`](https://crates.io/crates/winreg) | MIT |

### 服务端
| 组件 | 许可 |
|------|------|
| [`axum`](https://crates.io/crates/axum) | MIT |
| [`sqlx`](https://crates.io/crates/sqlx) | MIT OR Apache-2.0 |

### 桌面客户端（GUI）
| 组件 | 许可 |
|------|------|
| [`eframe`](https://crates.io/crates/eframe) | MIT OR Apache-2.0 |
| [`egui`](https://crates.io/crates/egui) | MIT OR Apache-2.0 |
| [`egui_extras`](https://crates.io/crates/egui_extras) | MIT OR Apache-2.0 |
| [`rfd`](https://crates.io/crates/rfd) | MIT |

### 命令行
| 组件 | 许可 |
|------|------|
| [`clap`](https://crates.io/crates/clap) | MIT OR Apache-2.0 |

---

## 3. 许可与合规说明

- 上述组件的**完整许可文本**随其源码分发，可在各 crate 仓库（<https://crates.io> / GitHub）查阅。
- MIT 与 Apache-2.0 许可均要求**保留版权与许可声明**——本文件即为该归属声明。
- 本产品不含 GPL/LGPL 传染性许可组件（截至本声明；若后续引入将以醒目方式另行标注）。
- `blake3` 采用 CC0-1.0 / Apache-2.0 / Apache-2.0 WITH LLVM-exception 三许可；`uuid`、`chacha20`、`aes-gcm` 等采用 Apache-2.0 / MIT 双许可，均可任选。

> 如需机器可读的完整许可清单，可在开发环境执行：
> ```
> cargo install cargo-about
> cargo about generate --config about.toml
> ```
> 自动生成第三方许可汇总。

---

*本声明随产品分发；如对归属或许可有疑问，请联系项目维护者。*

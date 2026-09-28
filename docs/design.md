# SecUnzip 设计文档

## 1. 定位

受控内容分发工具。解决：如何让文件只在被授权的人才能打开，同时防止顺手复制和扩散。

打包 → 授权用户 → 用户联网打开 → 内存解密为只读文件树，经回环 WebDAV 映射为盘符，不落盘。

| 目标用户 | 场景 |
|----------|------|
| 独立游戏开发者 | 游戏资源保护 |
| 企业 IT | 内部文档、培训视频交付 |
| 素材 / 课程卖家 | 数字内容分发 |

## 2. 核心设计

### 2.1 三条原则

1. 防君子不防小人 — 让顺手转发变得不方便
2. 黑盒执行 — 用户看不到密钥、看不到文件路径
3. 联网授权 — 必须联网获取密钥

### 2.2 用户标识

使用用户ID（手机号/邮箱/任意ID）而非机器ID：换设备也能用，管理直观，不需要客户端采集机器码。

## 3. 架构

### 3.1 组成

- 服务端：授权管理、密钥下发、临时申请审批，SQLite 存储
- 客户端 / 管理端：打包、打开、授权、申请（CLI + GUI）

### 3.2 打包产物

```
[文件头][加密数据]
```

文件头为明文，用 bincode 序列化 PackConfig，字段：magic、version、format、config、
data_offset、data_size、original_size、integrity_hash。

- 加密数据 = 源文件归档为 ZIP 后整体加密
- 文件ID = 产物内容的 MD5（内容寻址），**不写入文件头**，打开时由客户端重算
- `content_key` 不写入文件头，仅由服务端托管
- Remote 模式下文件头中的 key_derive 字段被清空

因为文件ID 可被任何拿到产物的人公开推导，服务端 register 必须防覆盖（见 §5.2）。

### 3.3 打开时的运行时形态

打开产物只有一条路径：校验完整性并解密后，把文件装载为内存中的只读文件树（`VirtualFS`）。

- 浏览：`src/runtime/mount.rs` 把该文件树以只读 WebDAV 在 `127.0.0.1` 随机端口服务出来，
  Windows 再用系统自带的 WebClient（`net use`）映射为盘符；盘符不写死，取 `Z:` 向下第一个空闲者。
- 落盘：仅在用户显式指定输出目录时解压到磁盘（`open -o`）。
- 产物头部的 `run_mode` 字段随 `PackConfig` 写入头部，但当前版本不解码后分支，打开行为与它无关；
  `RunMode` 的三个变体仅为保持头部编码稳定而保留（见 [technical.md](technical.md) §4）。

不用真实 ISO 挂载的原因：Windows 的 `Mount-DiskImage` 只能挂载磁盘上已存在的镜像文件，
满足它就必然先把解密后的明文写入磁盘，与「明文不落盘」直接冲突；真正的内存盘挂载需要
WinFsp / Dokan 之类的文件系统驱动或签名内核驱动，超出本项目的依赖范围。

## 4. 授权流程

直接授权：

```
管理员: secunzip grant file -u <用户ID> [-e 7d]
用户:   secunzip open  file -u <用户ID>
```

临时申请：

```
用户:   secunzip request file -u <用户ID> --days 3
管理员: secunzip approve file -u <用户ID>
```

## 5. 服务端

### 5.1 接口与鉴权

| 接口 | 鉴权 |
|------|------|
| [`/api/register`](../API.md#post-apiregister) | app_id 已存在时须持相同 secret |
| [`/api/key`](../API.md#post-apikey) | secret（管理员直取）或已授权的 user_id |
| [`/api/grant`](../API.md#post-apigrant)、[`/api/revoke`](../API.md#post-apirevoke)、[`/api/approve`](../API.md#post-apiapprove)、[`/api/deny`](../API.md#post-apideny) | 该文件的 secret |
| [`/api/request`](../API.md#post-apirequest) | 无，受该文件 allow_temp 限制 |
| [`/api/requests`](../API.md#post-apirequests)、[`/api/logs`](../API.md#post-apilogs) | 该文件的 secret |

请求/响应字段见 [../API.md](../API.md)。

### 5.2 关键约束

- **register 防覆盖**：app_id = 产物 MD5，可公开推导。若不校验，任何人都能覆盖他人文件的
  content_key / secret（令文件变砖或接管管理权）。因此 app_id 已存在时只接受相同 secret。
- **认证加密**：新打包使用 AES-256-GCM（密文含 16 字节认证 tag）；旧 AES-256-CBC 文件仍可解密，
  算法记录在文件头，解密时按头部派发。注意 `CryptoAlgo` 在头部按 bincode 变体序号编码，
  **新增算法必须追加在枚举末尾**，否则旧文件会被解析成错误的算法。
- **审计**：register / grant / revoke / key / request / approve / deny 全部落库。
- **数据库维护**：schema 用 `PRAGMA user_version` + 顺序迁移（语句幂等）；
  审计日志按「每个 app_id 最近 N 条」保留（默认 1000，启动时与每 6 小时各清理一次）；
  `secunzip-server --backup <文件>` 用 `VACUUM INTO` 在线生成一致性快照。

全部技术选型与实现方案见 [technical.md](technical.md)。

## 6. 技术栈

| 组件 | 技术 |
|------|------|
| 核心库 | Rust |
| 加密 | AES-256-GCM；兼容 AES-256-CBC、ChaCha20 |
| KDF | PBKDF2-HMAC-SHA256，10000 轮 |
| 压缩 | ZIP |
| 服务端 | Axum + SQLite |
| GUI | egui |

## 7. 安全边界

**信任锚**：服务端。它明文托管所有 content_key，服务端被攻破等于全部内容泄露。

**传输**：明文 HTTP，content_key 与 secret 会过网。仅可用于可信内网；对外必须前置 TLS 反向代理。

**授权即持久**：客户端为解密必然拿到 content_key，因此被授权者可提取密钥并永久离线解密与转发；
`revoke` 只能阻止今后的取钥。这是原则 1 的直接结果，不是缺陷。

**已知问题**：

- 文件ID 用 MD5。建议换 SHA-256；因内容寻址使 ID 公开，选择前缀碰撞理论上可行，
  但受 content_key 保密与 register 防覆盖限制，实际风险低。
- 文件头无 MAC。篡改明文头会改变文件ID，服务端查不到该 ID；密文有 GCM tag 保护。
- secret 比较非恒定时间；服务端无速率限制。

## 8. 路线图

- [x] 核心框架
- [x] CLI 命令
- [x] 服务端 API
- [x] GUI 客户端
- [x] 文件头部写入（Remote 模式下 key_derive 脱敏、content_key 不落文件）
- [x] 内存 EXE 执行（RunPE，`--features runpe` 开关化；默认构建不含）
- [x] 服务端安全加固（register 防覆盖、requests/logs 鉴权、审计日志、去 permissive CORS）
- [x] AES-256-GCM 认证加密
- [ ] TLS 传输；SHA-256 文件ID

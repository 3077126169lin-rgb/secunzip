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

原则 2 的落地程度：密钥由服务端托管、解密在内存完成，用户不直接接触密钥；黑盒自解压 EXE 已经可用——
GUI 与 CLI 都能产出，产物 runner 自查尾部标记后进入打开流程（见 §3.3、[technical.md](technical.md) §2）——
但真正的内存 EXE 执行（RunPE）仍未接入，没有调用点（见 §8）。

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
- `content_key` 是**打包时一次性确定的固定字符串**。打包界面的密钥流程选项（机器码 / 用户名 / 日期等）只在**打包机器上求值一次**
  （`gui/src/app.rs` `generate_key_from_flow`、`src/cli/commands.rs:42-44`），求值结果转成十六进制串即 `content_key`，交给服务端托管；
  打开时服务端原样下发该串，客户端固定按 `KeyNode::Input(KeySource::Literal(key_str))` 重建密钥（`src/runtime/loader.rs:101-104`）。
  **打开阶段不读取接收方的机器码、日期、IP 或用户名**，这些选项不构成对接收方环境的绑定，只决定打包时生成的密钥串本身。
- 头部 `PackConfig` 里的 `ip_whitelist`、`app_id`、`hash`、`allow_temp`、`run_mode` 会照原样序列化写入，但当前没有任何代码读取：
  `ip_whitelist` 从不校验；`hash` 不影响完整性校验（恒为 SHA-256，`src/packer/builder.rs:66`）；`run_mode` 不参与分支（见 §3.3）；
  `app_id` 在打包路径上恒为 `None`；`allow_temp` 用的是打包函数入参而非该字段（`src/core/types.rs:145-161`）。
  头部 `ip_whitelist` 是**装饰性字段**：接收方掌握客户端二进制，客户端校验可被绕过，因此 IP 限制只在服务端取钥时强制执行
  （服务端自己的 `apps.ip_whitelist` 列，见 §5.2）。该字段因 bincode 头部编码稳定性的要求保留，不删除。

因为文件ID 可被任何拿到产物的人公开推导，服务端 register 必须防覆盖（见 §5.2）。

### 3.3 打开时的运行时形态

打开 `.secunzip` 产物只有一条路径：校验完整性并解密后，把文件装载为内存中的只读文件树（`VirtualFS`）。

- 浏览：`src/runtime/mount.rs` 把该文件树以只读 WebDAV 在 `127.0.0.1` 随机端口服务出来，
  Windows 再用系统自带的 WebClient（`net use`）映射为盘符；盘符不写死，取 `Z:` 向下第一个空闲者。
- 落盘：仅在用户显式指定输出目录时解压到磁盘（`open -o`）。
- 产物头部的 `run_mode` 字段随 `PackConfig` 写入头部，但当前版本不解码后分支，打开行为与它无关；
  `RunMode` 的三个变体仅为保持头部编码稳定而保留（见 [technical.md](technical.md) §4）。
- EXE 黑盒产物并不走单独的运行时路径：`build_exe` 内嵌的头部把 `format` 固定写成 `SecUnzip`（`src/packer/builder.rs:159`），
  所以双击后同样是装载 VFS 浏览。`src/runtime/loader.rs:134-140` 中 `OutputFormat::Exe` 分支调用的 `execute_sandbox`
  （`loader.rs:215-247`）只在头部 format 字节为 0 时触发，打包器从不产出这种头部，该分支实际不可达；该函数本身是把文件
  **写入临时目录**再用资源管理器打开，**不是内存执行、也不是沙箱隔离**；真正的内存执行（RunPE）未接入（见 §8）。

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

`open` 只支持服务端认证：文件头为 `AuthMode::Local` 时直接报错拒绝（`src/cli/commands.rs:214-216`），GUI 也不产生此类文件。
文件头里的 `expire_at` 在生产打包路径上恒为 `None`（`src/cli/commands.rs:58`），因此 `src/runtime/loader.rs:64,93,166` 的过期检查
不会触发；实际生效的只有服务端下发的授权有效期，过期后服务端在取钥时拒绝并删除该授权（`server/src/main.rs:446-471`）。

## 5. 服务端

### 5.1 接口与鉴权

| 接口 | 鉴权 |
|------|------|
| [`/api/register`](../API.md#post-apiregister) | app_id 已存在时须持相同 secret |
| [`/api/key`](../API.md#post-apikey) | secret（管理员直取）或已授权的 user_id；两者都受该文件 ip_whitelist 限制 |
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
- **来源 IP 白名单**：`/api/register` 可带可选的 `ip_whitelist`，存于新增的 `apps.ip_whitelist` 列（JSON 数组，空 = 不限制）。
  `/api/key` 下发密钥前用 TCP 连接的对端地址（`ConnectInfo<SocketAddr>`）校验，不接受请求体自报的 IP；
  管理员 `secret` 路径同样受限——持 secret 者本就能重新注册改写白名单，绕行口子没有防锁定价值。
  匹配只实现 IPv4 单机（`203.0.113.7`）与 CIDR（`203.0.113.0/24`，前缀 0–32）：非法条目、IPv6 条目在注册时拒绝入库；
  库里若存在非法条目（只可能被人工改动）则忽略该条目并失败关闭；IPv6 来源地址在白名单非空时一律拒绝。
  未命中的取钥不返回密钥，并写 `action=key`、`success=false` 的审计记录。
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

头部枚举中声明但不可用的算法：`CryptoAlgo::Sm4Cbc` 由占位加密器在加解密时返回「SM4-CBC 尚未实现」
（`src/crypto/traits.rs:96-136`），`HashAlgo::Sm3` 在工厂处返回「SM3 尚未实现」（`src/crypto/hash.rs:65-72`）；
`CompressAlgo::SevenZ`、`TarZst`、`TarGz` 在打包与解包时报错，不再静默回退为 ZIP（`src/packer/compress.rs:124-158`）。
`KeyTransform::Concat` 没有可用语义，现在返回「变换未定义」错误（`src/key_derive/engine.rs:85-89`，
拼接由上层 `KeyNode::Concat` 处理）。这些路径一律返回中文错误，不再 panic，也不再静默替换算法。
`CryptoAlgo::XChaCha20` 已实现（`src/crypto/traits.rs:48-89`）。

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
- IP 白名单的可信度取决于服务端看到的来源地址：服务端绑 `0.0.0.0`，经 TCP 端口转发器或反向代理访问时，
  它看到的是**代理的地址**而不是客户端的地址，白名单只在服务端被直连、或代理保留源地址时才有效；
  否则所有请求会被当成来自同一个代理地址。头部 `PackConfig.ip_whitelist` 不参与此处校验（见 §3.2）。
  实测：服务端部署在软路由的容器里、由宿主上的 TCP 转发器转发 8090 时，它记录到的来源地址是
  `172.17.0.1`（Docker 网桥网关），而不是客户端的真实局域网地址。因此这种拓扑下白名单会拦下所有人，
  要按来源网段限制就必须让服务端被直连，或让代理保留源地址。

## 8. 路线图

- [x] 核心框架
- [x] CLI 命令
- [x] 服务端 API
- [x] GUI 客户端
- [x] 文件头部写入（Remote 模式下 key_derive 脱敏、content_key 不落文件）
- [ ] 内存 EXE 执行（RunPE，`--features runpe` 开关化；默认构建不含）。未接入：`run_pe_memory`（`src/runtime/runpe.rs:223`）无调用点；
  两条实际执行 EXE 的路径都先把文件落盘（`src/runtime/loader.rs:222-231` 写临时目录，`src/runtime/executor.rs:94-95` 写临时 EXE），
  且 `src/runtime/executor.rs` 的 `Executor` 整体无调用点
- [x] 服务端安全加固（register 防覆盖、requests/logs 鉴权、审计日志、去 permissive CORS）
- [x] AES-256-GCM 认证加密
- [ ] TLS 传输；SHA-256 文件ID

## 9. 未实现能力的替代方案

以下能力在本文其他位置已标注为未实现。这里记录若要补上，可行的手段与各自的代价，避免后来者重新推导一遍。

**环境指纹绑定（原设想的核心卖点之一）**

原设想是打开时按接收方的机器码、系统日期、IP 或域用户派生密钥。现已确认打开阶段完全不读这些值
（见 §3.3），密钥是打包时冻结的字符串。要让它在打开时真正生效，仅靠客户端自证没有意义——接收方控制
客户端，指纹可以伪造。可行做法只有两条：

- 注册式绑定：接收方先运行一次「打印本机指纹」的命令，把指纹交给管理员，管理员授权时录入服务端，
  取密钥时服务端比对。代价是多一个线下环节，且换机即失效。
- 短期或一次性密钥：服务端签发的密钥只在 N 分钟内有效、或只允许取用一次。代价是服务端要维护密钥状态
  与过期清理，但收益比指纹绑定更直接。

两者都不改变一个事实：密钥一旦下发就在接收方手里，指纹绑定与短期密钥都只是延后与收窄，不是阻止。

**文件内有效期 `expire_at`**

`runtime/loader.rs` 里有过期检查，但生产路径下该字段恒为空，检查不会触发（见 §3.3）。要让客户端侧过期
生效，前提是先接上 `AuthMode::Local` 离线模式：产物用本地密钥打包、打开时在本地派生，服务端不参与，
客户端过期检查才有意义。

而 Remote 模式（当前唯一可用的模式）下，服务端的授权有效期**已经完整工作**：授权、审批、过期拒绝
与过期清理都在服务端。所以对 Remote 而言该字段是冗余的——替代方案就是「用服务端的授权有效期，
不要指望文件里的过期」；若要支持离线分发，则需先实现 Local 模式，顺带让该字段与头部 `key_derive` 复活。

**运行模式 `RunMode`**

变体保留只是为了头部编码稳定。三种语义里，「内存只读挂载」正是当前的实际行为，落盘与否由 `open -o`
决定。因此可行的替代不是写新分支，而是把语义对齐到现状：当前默认行为即 `Document`，`-o` 即 `TempDir`，
`Sandbox`（内存执行）对应 §8 中未接入的 RunPE。这属于改名与文档工作，不需要新逻辑。

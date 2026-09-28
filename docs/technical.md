# 技术方案清单

本项目用到的所有技术选型与实现方案，按类别列出。每条给：**方案 / 实现位置 / 约束或理由**。

---

## 1. 语言与构建

| 方案 | 实现位置 | 说明 |
|------|---------|------|
| Rust，edition 2021 | [Cargo.toml](../Cargo.toml)、[server/Cargo.toml](../server/Cargo.toml)、[gui/Cargo.toml](../gui/Cargo.toml) | 核心库 + CLI、服务端、GUI 各为独立 crate |
| Windows 工具链 `x86_64-pc-windows-gnu`（MinGW） | 构建环境 | 链接系统自带 `msvcrt.dll`，因此不需要 VC++ Redistributable |
| Linux 工具链 native gnu | 服务端构建 | 产物为 ELF，受 glibc 版本约束 |
| 发布构建优化 | [Cargo.toml](../Cargo.toml) `[profile.release]` | `opt-level=3`、`lto=true`、`codegen-units=1`、`strip=true` |
| 功能开关 `runpe` | [Cargo.toml](../Cargo.toml) `[features]` | 内存 EXE 执行默认**不编译**，需显式 `--features runpe`；即便编译，`run_pe_memory` 也无调用点（`src/runtime/mod.rs:7-11`） |
| 控制台子系统 | [gui/src/main.rs](../gui/src/main.rs) | `#![cfg_attr(windows, windows_subsystem = "windows")]`，启动不弹控制台；`--register/--unregister` 用 `AttachConsole(ATTACH_PARENT_PROCESS)` 接回父终端输出 |
| 应用图标 | [build.rs](../build.rs)、[assets/icon.ico](../assets/icon.ico) | 编译期把图标作为 Windows 资源嵌入 exe：`build.rs` 只调用工具链自带的 `windres`（GNU 目标）或 `rc`（MSVC 目标），不引入额外 crate；找不到工具时只告警不中断构建。安装包另用 `IconFilename` 指向装好的 `.ico` |
| MSRV | 依赖声明 | 服务端 ≥ 1.71（tokio）、CLI ≥ 1.74（clap）、含 GUI 全量 ≥ 1.76（egui/eframe） |

## 2. 内容封装格式

| 方案 | 实现位置 | 说明 |
|------|---------|------|
| 自有扩展名 `.secunzip` | 全局 | 底层为「ZIP 归档整体加密」，外部无法直接识别为 ZIP |
| 文件结构 `[明文头部][加密数据]` | [src/packer/builder.rs](../src/packer/builder.rs) | 头部明文；数据段为加密后的归档 |
| 归档格式 ZIP | [src/packer/compress.rs](../src/packer/compress.rs) | 先归档多文件（保留路径），再整体加密 |
| 头部序列化 bincode | [src/core/config.rs](../src/core/config.rs) | `PackConfig` 用 bincode 序列化进头部；其余字段手工按小端写入 |
| 头部字段 | [src/core/config.rs](../src/core/config.rs) | `magic`、`version`、`format`、`config`、`data_offset`、`data_size`、`original_size`、`integrity_hash` |
| **枚举编码约束** | [src/core/types.rs](../src/core/types.rs) | 头部枚举按 bincode **变体序号**编码 → 新增算法必须**追加在枚举末尾**，插在中间会让旧文件解析成错误算法 |
| 完整性校验 SHA-256 | [src/packer/builder.rs](../src/packer/builder.rs) | 存的是「归档明文」的哈希，位于**明文头部** → 只能检测损坏，**不是 MAC**；算法恒为 SHA-256，不读 `PackConfig.hash`（`builder.rs:66`） |
| 头部字段只写不读 | [src/core/types.rs](../src/core/types.rs) `PackConfig` | `ip_whitelist`、`app_id`、`hash`、`allow_temp`、`run_mode` 照原样序列化写入头部，但**没有任何代码读取**：头部 `ip_whitelist` 不参与任何校验（实际生效的是服务端 `apps.ip_whitelist` 列，见 §6）；`hash` 不影响校验算法；`run_mode` 不分支；`app_id` 恒为 `None`（`src/cli/commands.rs:104`）；`allow_temp` 用的是打包函数入参而非该字段（`src/core/types.rs:137-161`） |
| 文件 ID = 产物 MD5 | [src/crypto/hash.rs](../src/crypto/hash.rs)、[src/cli/commands.rs](../src/cli/commands.rs) | 内容寻址，**不写入文件头**，打开时由客户端重算 |
| Remote 模式脱敏 | [src/packer/builder.rs](../src/packer/builder.rs) `header_config` | 头部 `key_derive` 字段被清空，`content_key` 绝不落文件（有单测 `content_key_and_plaintext_never_in_file`） |
| 黑盒 EXE 结构 | [src/packer/builder.rs](../src/packer/builder.rs) `build_exe` | `[runner][内嵌 .secunzip][trailer: offset u64 + MAGIC]`。runner = **打包进程自身的二进制**（`current_exe()`，`builder.rs:130`），GUI 与 CLI 都能产出黑盒 EXE：CLI 的 `pack --blackbox`（`src/cli/args.rs:34-36`）经 `pack_config` 把 `format` 置为 `Exe`（`src/cli/commands.rs:83-107`）；运行方在入口先只读末尾 16 字节做预筛，命中后交给库函数 `runtime::detect_self_extract` 判定并进入黑盒打开流程（`src/main.rs:77-96`、`src/runtime/selfextract.rs:19-33`），由 `cmd_blackbox` 取出内嵌产物、复用与 `open` 相同的联网取钥流程（`src/cli/commands.rs:176-194,199-267`）；argv 里出现已知子命令时仍按普通 CLI 解析（`src/main.rs:64-71`），工具自身功能不受影响。GUI 路径不变，仍是启动时自查尾部标记（`gui/src/app.rs:116-117`）。**内嵌**的 `.secunzip` 头部 `format` 由 `build_secunzip` 固定写成 `SecUnzip`（`builder.rs:159`），故双击后同样装载 VFS 浏览。取不到自身路径时回退到 `assets/runtime_stub.exe`（11 字节占位，`builder.rs:133`） |
| 头部版本号 | [src/core/config.rs](../src/core/config.rs) | `FORMAT_VERSION` 常量，供将来格式升级判别 |

## 3. 加密与密钥

| 方案 | 实现位置 | 说明 |
|------|---------|------|
| 默认加密 **AES-256-GCM** | [src/crypto/aes.rs](../src/crypto/aes.rs) `Aes256GcmEncryptor` | 认证加密（密文尾部 16 字节 tag），可检测篡改；nonce 12 字节 |
| 兼容 AES-256-CBC | [src/crypto/aes.rs](../src/crypto/aes.rs) `Aes256CbcEncryptor` | PKCS7 填充，IV 16 字节；旧文件仍可解密 |
| 兼容 ChaCha20 / XChaCha20 | [src/crypto/chacha.rs](../src/crypto/chacha.rs)、[src/crypto/traits.rs](../src/crypto/traits.rs) `XChaCha20Encryptor` | 两者都是流密码；XChaCha20 的 `iv_len()` 为 12，派生的 12 字节 IV 放到 24 字节 XNonce 前半、后半补零（`traits.rs:63-66`），磁盘布局与 ChaCha20 一致 |
| 算法派发 | [src/crypto/traits.rs](../src/crypto/traits.rs) `create_encryptor` | 解密时按**头部记录的算法**选择实现 → 新算法不破坏旧文件 |
| 算法覆盖不完整 | [src/crypto/traits.rs](../src/crypto/traits.rs)、[src/crypto/hash.rs](../src/crypto/hash.rs)、[src/packer/compress.rs](../src/packer/compress.rs) | 已实现：`Aes256Cbc` / `Aes256Gcm` / `ChaCha20` / `XChaCha20`。未实现：`CryptoAlgo::Sm4Cbc` 由 `UnsupportedEncryptor` 在加解密时返回「SM4-CBC 尚未实现」（`traits.rs:96-136`），`HashAlgo::Sm3` 在工厂处返回「SM3 尚未实现」（`hash.rs:65-72`），`CompressAlgo::SevenZ` / `TarZst` / `TarGz` 在打包与解包时返回错误（`compress.rs:124-158`）。三处工厂都不写 `_ =>` 兜底，新增变体必须显式处理；既不再 panic，也不再静默替换算法 |
| 密钥派生 PBKDF2-HMAC-SHA256 | [src/crypto/keywrap.rs](../src/crypto/keywrap.rs) | 10000 轮；输出拼接为 `key \|\| iv`（GCM 12B / CBC 16B） |
| 密钥派生流程 KeyNode 树 | [src/key_derive/engine.rs](../src/key_derive/engine.rs) | 节点类型：`Input` / `Transform`（`Hash` / `TakeFirst` / `TakeLast` / `AddSalt` / `Base64Encode` / `Base64Decode`）/ `Conditional` / `Concat`。**只在打包机器上求值一次**：`generate_key_from_flow`（`engine.rs:125`）把求值结果转成十六进制串作为 `content_key`；打开时不再求值该树 |
| 派生来源 | [src/key_derive/sources.rs](../src/key_derive/sources.rs) | `MachineGuid`（Windows 注册表，`sources.rs:25-39`）、`LocalIp`（连接 `8.8.8.8:80` 取 OS 选定出口网卡的地址，`sources.rs:49-64`）、`CurrentDate`（本机时钟 `YYYYMMDD`，`sources.rs:68-71`）、`DomainUser`（**只读 `USERNAME` / `USER` 环境变量，不读域名**，`sources.rs:74-89`）、`Salt`、`Literal`；`SteamId` 直接返回「尚未实现」错误（`sources.rs:14-19`）。这些来源都只在**打包时**参与求值，打开时一律不读取 |
| `KeyTransform::Concat` 未定义 | [src/key_derive/engine.rs](../src/key_derive/engine.rs) `apply_transform` | 返回「密钥派生变换 Concat 未定义」错误（`engine.rs:85-89`），不再原样返回冒充变换成功；拼接由上层 `KeyNode::Concat` 承担 |
| content_key 生成 | [src/cli/commands.rs](../src/cli/commands.rs) `pack_file` | 优先用自定义流程，否则两个 UUIDv4 拼接（高熵） |
| 管理密钥 secret | [src/cli/commands.rs](../src/cli/commands.rs) `pack_file` | UUIDv4 |
| 密钥托管 | [server/src/main.rs](../server/src/main.rs) | `content_key` 只存服务端，联网验证授权后下发 |

## 4. 运行时形态

| 方案 | 实现位置 | 说明 |
|------|---------|------|
| 内存解压 / 只读浏览（默认） | [src/runtime/loader.rs](../src/runtime/loader.rs)、[src/runtime/vfs.rs](../src/runtime/vfs.rs) | 不落盘；虚拟文件系统列举与读取。打包器写出的头部 `format` 恒为 `SecUnzip`（`builder.rs:159`），黑盒 EXE 内嵌的也是这种头部，故实际打开都走此路径 |
| 解压到磁盘（可选） | [src/cli/commands.rs](../src/cli/commands.rs) `cmd_open -o` | 显式指定输出目录时才落盘 |
| 运行模式 `RunMode` | [src/core/types.rs](../src/core/types.rs) | 随 `PackConfig` 用 bincode 写入产物头部；**当前版本不读取、不分支**（打开行为与它无关），三个变体仅为保持头部编码稳定，`Document` 无任何引用 |
| 只读 WebDAV 挂载 | [src/runtime/mount.rs](../src/runtime/mount.rs) | 把内存 VirtualFS 在 `127.0.0.1` 随机端口以只读 WebDAV 服务出来；Windows 用系统自带 WebClient（`net use`）映射为盘符，非 Windows 只启动服务（`#[cfg(not(windows))]`） |
| 不用真实 ISO 挂载 | 取舍 | `Mount-DiskImage` 只能挂载磁盘上已存在的镜像文件，用它必须先写明文到磁盘，与不落盘冲突；真正的内存盘需 WinFsp / Dokan 等文件系统驱动，超出零第三方运行时范围。故改用回环 WebDAV |
| 自解压 EXE 运行 | [src/runtime/selfextract.rs](../src/runtime/selfextract.rs) | 识别尾部标记并解出内嵌 `.secunzip`；**已接入 CLI 与 GUI**：CLI 入口先只读末尾 16 字节预筛、命中后调 `detect_self_extract`（`src/main.rs:15-24,77-96`），GUI 启动时同样自查（`gui/src/app.rs:116-117`）；带已知子命令时仍按普通 CLI 解析 |
| 内存 EXE 执行（RunPE） | [src/runtime/runpe.rs](../src/runtime/runpe.rs) | **未接入**：`run_pe_memory`（`runpe.rs:223`）在代码中无调用点，且 `#[cfg(all(windows, feature = "runpe"))]` 门控、默认不编译（`src/runtime/mod.rs:7-11`、`Cargo.toml:66-69`） |
| 沙箱执行 | 无 | 没有沙箱隔离实现。`execute_sandbox`（`src/runtime/loader.rs:215-247`）会把文件**写入临时目录**再调 `explorer.exe` 打开，但它只挂在 `OutputFormat::Exe` 分支（`loader.rs:134-140`），而打包器写出的头部 `format` 恒为 `SecUnzip`（`builder.rs:159`），该分支实际不可达 |
| 手动打开/定位文件 | [src/runtime/loader.rs](../src/runtime/loader.rs) | **不可达**：`execute_sandbox` 里的 `explorer.exe` 调用（`loader.rs:234-240`）随 `OutputFormat::Exe` 分支一起不可达（见上行）；`src/runtime/executor.rs` 内含 `explorer` / `xdg-open` 分支，但 `Executor` 全类型无调用点，不参与运行 |

## 5. 授权模型

| 方案 | 实现位置 | 说明 |
|------|---------|------|
| 按用户 ID 授权 | 全局 | 手机号/邮箱/任意 ID；非机器码，换设备仍可用 |
| 联网取钥 | [src/cli/commands.rs](../src/cli/commands.rs) `cmd_open` | 打开时必须请求服务端；服务端地址取自文件头 `auth_mode` |
| 认证模式 `AuthMode` | [src/core/types.rs](../src/core/types.rs) | 只有 `Remote(服务端URL)` 可用：`open` 遇到 `AuthMode::Local` 直接报错「本地模式不支持」（`src/cli/commands.rs:214-216`），GUI 也不产生 Local 文件 |
| 授权有效期 | [server/src/main.rs](../server/src/main.rs) `parse_expire` | 实际生效的只有**服务端 grant** 的 `expires_at`：永久 / `Nd`（N 天后）/ `YYYYMMDD`；过期在被取钥时拒绝并删除该授权（`server/src/main.rs:446-471`）。**文件头 `expire_at` 未生效**：生产打包恒写 `None`（`src/cli/commands.rs:58`），`src/runtime/loader.rs:64,93,166` 的检查永不触发 |
| content_key 与环境无关 | [src/runtime/loader.rs](../src/runtime/loader.rs) `extract_with_key` | 打开时服务端下发打包时确定的 `content_key` 字符串，客户端固定按 `KeyNode::Input(KeySource::Literal(key_str))` 重建密钥（`loader.rs:101-104`），不读取接收方的机器码 / 日期 / IP / 用户名 |
| 密钥派生时机 | [src/key_derive/engine.rs](../src/key_derive/engine.rs) `generate_key_from_flow` | KeyNode 树只在**打包机器上求值一次**（只有 GUI 会构造流程树：`gui/src/app.rs:377`；CLI 直接写死 `Literal` 字面量节点，`src/cli/commands.rs:55`），结果转成十六进制串即 `content_key` 并托管到服务端 |
| 临时申请 + 审批 | [server/src/main.rs](../server/src/main.rs) `/api/request`、`/api/approve`、`/api/deny` | 打包时 `allow_temp` 决定是否开放；同一用户已有 pending 时去重 |
| 管理操作凭据 | `.secret` 文件 / `SECUNZIP_SECRET` | `grant`/`revoke`/`approve`/`deny`/`requests`/`logs` 均需该文件的 secret |
| 默认不落盘 | [src/runtime/vfs.rs](../src/runtime/vfs.rs) | 降低顺手复制 |

## 6. 服务端

| 方案 | 实现位置 | 说明 |
|------|---------|------|
| Axum 0.7 + tokio | [server/src/main.rs](../server/src/main.rs) | 单文件服务端，9 个 POST 接口 + `GET /healthz` 存活探针；未注册 `GET /`，客户端探测根路径时收到的是 Axum 的 404 |
| SQLite（**bundled**） | [server/Cargo.toml](../server/Cargo.toml) | sqlx 的 `sqlite` 特性开启 `libsqlite3-sys/bundled` → SQLite 源码编译进二进制，**不依赖系统 libsqlite3** |
| 表结构 | [server/src/main.rs](../server/src/main.rs) `MIGRATIONS` | `apps`（含 `ip_whitelist` 列，JSON 数组文本）、`grants`、`requests`、`audit_logs` |
| 索引 | [server/src/main.rs](../server/src/main.rs) `MIGRATIONS` | `idx_requests_app_status`、`idx_audit_app`（除主键外的二级索引） |
| **迁移机制** | [server/src/main.rs](../server/src/main.rs) `migrate` | `PRAGMA user_version` + 顺序迁移表，当前 `SCHEMA_VERSION = 2`（`main.rs:110`）；每级语句幂等（`IF NOT EXISTS`）；v2 给 `apps` 补 `ip_whitelist` 列（`main.rs:155-159`）；库版本高于程序时拒绝启动 |
| **来源 IP 白名单** | [server/src/main.rs](../server/src/main.rs) `check_source_ip`、`load_whitelist` | `/api/key` 在两种取钥路径之前用 `ConnectInfo<SocketAddr>` 的 TCP 对端地址校验（`main.rs:483-507`），不接受请求体自报的 IP；`/api/register` 注册时用 `valid_ip_entry` 拒绝非法与 IPv6 条目（`main.rs:790-793`）。未设或空数组 = 不限制；未命中返回「来源 IP … 不在该文件的 IP 白名单内，拒绝下发密钥」并写 `action=key`、`success=false` 审计；管理员路径同样受限；只实现 IPv4 单机与 CIDR，库中无法解析的存量值失败关闭（`main.rs:799-859`）。头部 `PackConfig.ip_whitelist` 不参与此处校验 |
| 审计日志 | [server/src/main.rs](../server/src/main.rs) `log_audit` | register/grant/revoke/key/request/approve/deny 全部落库 |
| **审计保留策略** | [server/src/main.rs](../server/src/main.rs) `prune_audit` | 每个 `app_id` 只保留最近 N 条（默认 1000，`SECUNZIP_AUDIT_KEEP` 可覆盖）；启动时一次 + 每 6 小时一次 |
| **备份** | [server/src/main.rs](../server/src/main.rs) `--backup` | 用 `VACUUM INTO` 生成一致性快照，**可在服务端运行中执行** |
| 配置项 | [server/src/main.rs](../server/src/main.rs) | `DATABASE_URL`（默认 `sqlite:secunzip.db?mode=rwc`，目录不存在自动创建）、`SECUNZIP_PORT`（默认 8090）、`SECUNZIP_AUDIT_KEEP` |
| register 防覆盖 | [server/src/main.rs](../server/src/main.rs) `register_app` | app_id 已存在时**只接受相同 secret** 的更新；否则拒绝（app_id = MD5 可公开推导，不校验则任何人可覆盖他人文件） |
| requests / logs 鉴权 | [server/src/main.rs](../server/src/main.rs) `list_requests`、`list_logs` | 需该文件 secret，且**只返回本文件**的数据 |
| CORS | 未启用 | 客户端均为原生程序，不需要跨域 |
| 传输层 | 明文 HTTP | 无 TLS —— 见 §10 |

## 7. 客户端

| 方案 | 实现位置 | 说明 |
|------|---------|------|
| CLI（clap 4.5） | [src/cli/](../src/cli) | `pack` / `open` / `grant` / `revoke` / `request` / `requests` / `approve` / `deny` |
| GUI（egui / eframe 0.29） | [gui/src/](../gui/src) | 原生窗口（非 webview）；按功能拆分：[theme.rs](../gui/src/theme.rs) 设计系统、[icons.rs](../gui/src/icons.rs) 矢量图标（手搓绘制，无 emoji / 无图标字体）、[model.rs](../gui/src/model.rs) 状态、[monitor.rs](../gui/src/monitor.rs) 后台监控、[views/](../gui/src/views) 各页面、[app.rs](../gui/src/app.rs) 业务逻辑 |
| GUI 中文字体 | [gui/src/main.rs](../gui/src/main.rs) | 依次尝试 `msyh.ttc` / `simsun.ttc` / `simhei.ttf` |
| HTTP 客户端 | [gui/src/api.rs](../gui/src/api.rs)、[src/cli/commands.rs](../src/cli/commands.rs) | reqwest |
| 后台监控线程 | [gui/src/monitor.rs](../gui/src/monitor.rs) `Monitor` | 独立线程做服务器连通性探测 + 待审批自动拉取（4 秒周期），不阻塞 UI |
| 本地配置 | `%USERPROFILE%\Documents\SecUnzip\` | `config.json`（用户ID、服务端地址）、`packed.json`（打包记录） |
| 管理密钥存放 | 产物同目录 `xxx.secret` | 可用 `SECUNZIP_SECRET` 覆盖 |
| 文件关联注册 | [gui/src/register.rs](../gui/src/register.rs) | 调 `reg.exe` 写 `HKCU\Software\Classes`，再调 `SHChangeNotify` 通知资源管理器刷新 |
| 单实例互斥 | [installer.iss](../installer.iss) `AppMutex` | 安装/卸载时避免与运行中的客户端冲突 |

## 8. 部署与打包

| 方案 | 实现位置 | 说明 |
|------|---------|------|
| Windows 安装包 | [installer.iss](../installer.iss) | **Inno Setup 6**；`lzma2/max` 固实压缩、modern 向导、x64、`PrivilegesRequired=lowest`（按用户安装，不需管理员）；可选注册文件关联；输出 `output/SecUnzip-Setup.exe` |
| 安装包**不装依赖** | [installer.iss](../installer.iss) | 无任何运行时/redist 安装步骤 —— 因为打包的二进制本身无第三方依赖 |
| 一键部署脚本 | [deploy/install-windows.ps1](../deploy/install-windows.ps1)、[deploy/install-linux.sh](../deploy/install-linux.sh) | 生成服务化脚本（Windows 生成 NSSM 脚本、Linux 生成 systemd unit） |
| Docker | [deploy/Dockerfile](../deploy/Dockerfile)、[deploy/docker-compose.yml](../deploy/docker-compose.yml) | 构建镜像 `rust:1.85`；运行阶段 `debian:bookworm-slim`；数据卷 `/data`，经 `DATABASE_URL` 指向 |
| 本机实测形态 | — | Linux 服务端实际部署在 x86_64 Ubuntu 24.04（glibc 2.39）上运行 |

## 9. 依赖与平台

| 方案 | 说明 |
|------|------|
| 零第三方运行时 | 三个 exe 只 import 系统 DLL；**不需要 VC++ Redistributable**、不需要系统 libsqlite3、不需要 Python/Node/JRE |
| Windows 版本要求 | 需含相应 API set（`api-ms-win-core-path-l1-1-0.dll` 为 Win8+） |
| Linux glibc 约束 | 在 glibc 2.39 上构建 → 目标机过低无法运行；兼容旧发行版需改用 musl 目标 |
| 平台验证状态 | CLI/GUI 仅 Windows 构建并测试；服务端 Windows 构建 + Linux 部署运行；macOS 未验证 |

## 10. 安全边界（如实列出，非缺陷即约束）

| 项 | 说明 |
|----|------|
| 传输明文 | 无 TLS；`content_key` 与 `secret` 会过网 → 仅可用于可信内网，对外必须前置 TLS 反代 |
| 授权即持久 | 客户端为解密必然拿到 `content_key`，被授权者可永久离线解密与转发；`revoke` 只阻止今后的取钥 |
| 服务端为信任锚 | `content_key` 与 `secret` 在库中**明文存储**；数据库文件泄露等于全部内容泄露 |
| 文件 ID 用 MD5 | 可公开推导；实际风险受 `content_key` 保密 + register 防覆盖限制。建议后续换 SHA-256 |
| 文件头无 MAC | 篡改明文头会改变文件 ID（服务端查不到该 ID）；密文由 GCM tag 保护 |
| secret 比较非恒定时间 | SQL 字符串比较 |
| 服务端无限流 | 依赖 secret 为 UUID 的高熵 |
| 无多实例协调 | 多个服务端进程指向同一库时，仅靠 SQLite 自身文件锁 |

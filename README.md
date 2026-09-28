# secunzip

受控内容分发工具。把文件打包成 `.secunzip`，接收方必须联网、且被授权，才能打开。

内容先归档为 ZIP，再整体加密。加密密钥由服务端托管，打开时联网获取；默认在内存中解压浏览，不落盘。

```
secunzip pack ./docs -o docs.secunzip --server http://192.168.1.10:8090
secunzip grant docs.secunzip -u alice@example.com
secunzip open  docs.secunzip -u alice@example.com
```

## 仓库结构

| 路径 | 内容 |
|------|------|
| [src/](src/) | 核心库 + CLI：[core/](src/core/) 类型与产物格式、[crypto/](src/crypto/) 加密与哈希、[key_derive/](src/key_derive/) 密钥派生、[packer/](src/packer/) 打包、[runtime/](src/runtime/) 加载与挂载、[cli/](src/cli/) 命令行 |
| [server/](server/) | 授权服务端（Axum + SQLite），全部逻辑在 [src/main.rs](server/src/main.rs) |
| [gui/](gui/) | 图形客户端（egui）：[theme.rs](gui/src/theme.rs) 设计系统、[icons.rs](gui/src/icons.rs) 手搓矢量图标、[model.rs](gui/src/model.rs) 状态与持久化、[monitor.rs](gui/src/monitor.rs) 后台监控、[views/](gui/src/views/) 各页面、[app.rs](gui/src/app.rs) 业务逻辑、[api.rs](gui/src/api.rs) HTTP 调用 |
| [tests/](tests/) | 集成测试 |
| [testdata/](testdata/) | 示例数据（测试当前自建临时文件，此目录未被引用） |
| [docs/](docs/) | [设计文档](docs/design.md)、[技术方案清单](docs/technical.md) |
| [deploy/](deploy/) | 部署脚本与 Docker，见 [deploy/README.md](deploy/README.md) |
| [assets/](assets/) | 打包用的运行时占位资源 |
| [installer.iss](installer.iss) | Inno Setup 安装包脚本 |

## 构建

需要 Rust stable。

```
cargo build --release
cd server && cargo build --release
cd gui    && cargo build --release
```

产物：`target/release/secunzip`、`server/target/release/secunzip-server`、`gui/target/release/secunzip-gui`。

## 命令

```
secunzip pack <源路径...> -o <输出> --server <地址> [--allow-temp]
secunzip open <文件> -u <用户ID> [-o <解压目录>]
secunzip grant <文件> -u <用户ID> [-e <有效期>]
secunzip revoke <文件> -u <用户ID>
secunzip request <文件> -u <用户ID> [--days N] [--message <说明>]
secunzip requests <文件>
secunzip approve <文件> -u <用户ID> [-e <有效期>]
secunzip deny <文件> -u <用户ID>
```

`-e/--expires` 接受 `7d`（N 天后）或 `YYYYMMDD`，省略为永久。
命令定义见 [src/cli/args.rs](src/cli/args.rs)。

`pack` 产出两个文件：

- `xxx.secunzip` — 加密产物，可对外分发
- `xxx.secret` — 该文件的管理密钥，用于 grant/revoke/approve/deny，不要与产物一起发出去

管理端命令默认从产物同目录的 `.secret` 读取密钥，也可用环境变量 `SECUNZIP_SECRET` 覆盖。

## 服务端

```
cd server
./secunzip-server                       # 监听 0.0.0.0:8090
SECUNZIP_PORT=9000 ./secunzip-server    # 换端口
```

数据存工作目录下的 `secunzip.db`（SQLite，启动时自动建表）。服务端源码见 [server/src/main.rs](server/src/main.rs)，
接口见 [API.md](API.md)，部署脚本见 [deploy/README.md](deploy/README.md)。

## GUI

```
cd gui && cargo run
```

- 「打开」页：拖入 `.secunzip` 浏览内容；未加载文件时列出本机打包过的文件
- 「打包」页：选择源、输出路径、服务端地址
- 「设置」页：用户ID、服务端地址

配置位于 `%USERPROFILE%\Documents\SecUnzip\`（`config.json`、`packed.json`）。GUI 源码见 [gui/src/](gui/src/)。

## 平台支持

| 组件 | Windows | Linux x86_64 | macOS |
|------|---------|--------------|-------|
| CLI | 构建并测试 | 未验证 | 未验证 |
| 服务端 | 构建 | 构建、部署并运行（Ubuntu 24.04 / glibc 2.39） | 未验证 |
| GUI | 构建 | 未验证 | 未验证 |

CLI 与 GUI 中含有 `#[cfg(not(windows))]` 分支，可编译到非 Windows 平台，但尚未验证。

以下功能仅 Windows 可用：

- 读取 MachineGuid 作为密钥派生来源（`winreg`）
- 盘符挂载
- `--features runpe` 的内存执行
- 黑盒自解压 EXE（产物为 PE，需 Windows 打开）

`.secunzip` 产物本身与平台无关，任何平台的客户端都能打开。

## 运行要求

- 服务端：一个可写目录（存放 `secunzip.db`）和一个监听端口（默认 8090），无外部数据库或中间件
- 客户端：能通过 HTTP 访问服务端即可
- GUI：需要桌面窗口系统（eframe 原生渲染，不依赖浏览器）
- 构建：Rust stable；Windows 上使用 `x86_64-pc-windows-gnu`（mingw）

## 依赖与安装

三个可执行文件都是自包含的原生二进制，**不需要任何第三方运行时**：

- Windows：只依赖系统 DLL（kernel32 / msvcrt / ws2_32 / bcrypt / crypt32 等），**不需要 VC++ Redistributable**（MinGW 工具链，链接系统自带的 `msvcrt.dll`）
- SQLite 以 `bundled` 方式编译进服务端二进制，**不需要系统安装 libsqlite3**
- 不需要 Python / Node / JRE / 外部数据库

已知约束：

- Windows：需要含相应 API set 的系统（约 Win8+）
- Linux 服务端受 **glibc 版本**约束 —— 在 glibc 2.39（Ubuntu 24.04）上构建，目标机过低会无法运行；要兼容旧发行版请改用 musl 目标
- macOS：未验证

### 安装包

[installer.iss](installer.iss) 是 [Inno Setup 6](https://jrsoftware.org/isinfo.php) 脚本，`iscc installer.iss` 产出 `output/SecUnzip-Setup.exe`：

- 安装 GUI、CLI、服务端三个可执行文件，以及 [deploy/](deploy/) 脚本与文档
- 创建开始菜单与（可选）桌面快捷方式
- 可选注册 `.secunzip` 文件关联（写 HKCU）
- 按用户安装（`PrivilegesRequired=lowest`），不需要管理员

**安装包不安装任何依赖** —— 因为它打包的二进制本身没有依赖（见上）。

## 安全

明文 HTTP 传输，`content_key` 与 `secret` 会过网，只应部署在可信内网；对外必须前置 TLS 反向代理。

服务端明文托管所有 `content_key`，是唯一的信任锚。

客户端为解密必然拿到密钥，因此**授权一次即可永久离线解密**；`revoke` 只阻止今后的取钥，
无法追回已泄露的密钥。

威胁模型与已知问题见 [docs/design.md](docs/design.md)。

## 测试

```
cargo test                # 48 个
cd server && cargo test
```

见 [TESTING.md](TESTING.md)。

## 文档

- [API.md](API.md) — 服务端接口
- [TESTING.md](TESTING.md) — 测试范围与手工验证
- [docs/design.md](docs/design.md) — 设计：架构、产物格式、安全边界
- [docs/technical.md](docs/technical.md) — 技术方案清单（全部选型与实现方案）
- [ATTRIBUTION.md](ATTRIBUTION.md) — 第三方归属
- [deploy/README.md](deploy/README.md) — 服务端部署

## License

[MIT](LICENSE)

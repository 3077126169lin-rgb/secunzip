# 贡献指南

本项目是课程作业，完成后**不再维护**：不承诺审查或合并 PR，也不承诺回复 issue。
下面的约定用于说明代码是如何构建与验证的，而不是在征集贡献。

## 构建

仓库是一个 Cargo workspace，三个成员 crate 共用一份 `Cargo.lock` 与根目录的 `target/`：

| crate | 产物（都在根目录 `target/release/` 下） |
|------|------|
| 根目录（核心库 + CLI） | `secunzip.exe` |
| [server/](server/) | `secunzip-server.exe` |
| [gui/](gui/) | `secunzip-gui.exe` |

```
cargo build --release --workspace            # 一次构建全部
cargo build --release -p secunzip-gui        # 只构建其中一个
```

Windows 上使用 MinGW 工具链 `stable-x86_64-pc-windows-gnu`，`C:\msys64\mingw64\bin` 必须在 `PATH` 中：
`cc` 用于以 `bundled` 方式编译 SQLite，`windres` / `rc` 用于 [build.rs](build.rs) 嵌入图标。

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu
$env:PATH = "C:\msys64\mingw64\bin;$env:PATH"
cargo +stable-x86_64-pc-windows-gnu build --release
```

`--features runpe` 默认关闭，仅在需要验证内存执行时启用。

## 测试

```
cargo test --workspace           # 全部，73 个
cargo test -p secunzip           # 仅核心库与 CLI，52 个
cargo test -p secunzip-server    # 仅服务端，21 个
```

用例分布与重点回归项见 [TESTING.md](TESTING.md)。改动加密、打包、密钥派生或服务端鉴权后，
请按该文件的手工验证表逐项确认。

## 代码风格

- 提交前执行 `cargo fmt`，格式化的差异不要混进功能提交。
- 三个 crate 都必须 clippy 干净：在各目录下执行 `cargo clippy --all-targets`，不新增警告。
- 注释、CLI 输出、GUI 文案统一用中文，不引入 emoji。
- 新增加密算法必须追加在 `CryptoAlgo` 枚举末尾，否则旧产物会被按错误的算法解析
  （见 [docs/design.md](docs/design.md) 5.2）。

## 提交

提交信息是一行中文，写成 `类别：做了什么`，类别沿用仓库现有的几个：

```
加密：新增 AES-256-GCM 认证加密
服务端：register 增加防覆盖校验
文档：README 增加逐文件清单
```

一个提交只做一件逻辑上的事。无关的格式化或重命名不要和功能改动放在同一个提交里。

## 行为变更

[docs/design.md](docs/design.md) 是行为的事实来源。凡是改变产物格式、密钥派生、鉴权规则或安全边界的改动，
先改设计文档再改代码，并在 PR 里说明理由。涉及 HTTP 接口的改动同时更新 [API.md](API.md)。

## 安全

威胁模型、已知并接受的风险与漏洞报告方式见 [SECURITY.md](SECURITY.md)。

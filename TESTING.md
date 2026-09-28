# 测试

## 运行

```
cargo test                      # 核心库 + 集成测试，52 个
cd server && cargo test         # 服务端黑盒测试，21 个（真实拉起进程打 HTTP 接口）
cd gui    && cargo build
```

CI（[.github/workflows/ci.yml](.github/workflows/ci.yml)）在每次推送与 PR 上跑这几条，外加
`cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings`。

## 分布

| 位置 | 数量 | 覆盖 |
|------|------|------|
| `src/**`（`#[cfg(test)]`） | 19 | 密钥派生引擎、VFS、WebDAV 挂载、自解压识别 |
| [tests/crypto_test.rs](tests/crypto_test.rs) | 10 | AES-CBC/GCM 往返、ChaCha20、错密钥、GCM 篡改检测、哈希、KDF |
| [tests/key_derive_test.rs](tests/key_derive_test.rs) | 10 | 派生流程节点求值、条件分支、确定性 |
| [tests/packer_test.rs](tests/packer_test.rs) | 6 | 打包/解包往返、有效期、ChaCha20、错密钥、服务端密钥解包、info |
| [tests/security_test.rs](tests/security_test.rs) | 3 | content_key 不落文件、注册失败不留死文件、黑盒 EXE 自解压 |
| [tests/robustness_test.rs](tests/robustness_test.rs) | 4 | 所有截断长度、采样位翻转、随机与边界填充输入都不得 panic |
| [server/tests/api_test.rs](server/tests/api_test.rs) | 21 | 服务端鉴权、注册防覆盖、多应用隔离、迁移与审计保留、备份、启动失败路径 |

`src/**` 的 19 个：

- `key_derive::engine`（2）：同一流程密钥确定；不同流程密钥不同
- `runtime::vfs`（10）：构建、列目录、子目录、深层嵌套、读文件、读缺失、目录当文件读、文本/图片判定、总大小、空 VFS
- `runtime::mount`（5）：WebDAV OPTIONS/PROPFIND/GET/404、路径规范化
- `runtime::selfextract`（2）：识别并解出内嵌产物、拒绝非自解压文件

## 服务端测试怎么写的

[server/tests/api_test.rs](server/tests/api_test.rs) 用 `CARGO_BIN_EXE_secunzip-server` 拉起真实的
`secunzip-server` 进程，每个用例独占一个临时目录和一个空闲端口，请求用 `std::net::TcpStream`
手写 HTTP/1.1 发出，不引入 HTTP 客户端依赖。断言既看 HTTP 响应，也直接查 SQLite 里的落库结果。

覆盖到的关键性质：

- 注册防覆盖：`app_id` 等于文件 MD5、公开可推，但猜不到 `secret` 时注册被拒，且原 `content_key` 不被改写
- 鉴权：错 `secret` 无法 grant / revoke，也看不到 requests 与 logs
- 隔离：拿 A 文件的 `secret` 查 A 之外的申请与日志会被拒；正确 `secret` 只返回本文件的数据
- 授权生命周期：未授权取不到钥、授权后取到、吊销后立刻失效、过期授权被拒并顺手删除该条授权
- 临时申请：未开启 `allow_temp` 时拒收、重复提交不产生第二条待审批、审批通过按 `need_days` 授权、拒绝不授权
- 运维路径：重启后数据仍在且迁移幂等、`DATABASE_URL` 的多级父目录自动创建、审计日志按
  `SECUNZIP_AUDIT_KEEP` 保留每个应用最新的 N 条、`--backup` 产出的快照可读、
  库 schema 高于程序时拒绝启动、端口被占用时报错退出

## 解析健壮性怎么测的

[tests/robustness_test.rs](tests/robustness_test.rs) 是 cargo-fuzz 的确定性替代（不依赖 nightly 与
libfuzzer，每次运行结果一致）：先造一个正常产物，再对它做三类破坏，用 `catch_unwind` 断言解析器
只返回错误、绝不 panic。

- 逐个长度截断（从 0 到完整长度，一个不落）
- 每 64 字节采样一个位置，逐位翻转
- 固定种子的伪随机字节、全 0 与全 0xFF 填充、空输入

这个测试在引入时立刻抓到一个真实缺陷：`PackHeader::from_bytes` 会用文件里的 `hash_len` 直接切片，
而前面只校验了 32 字节固定尾字段，于是被截断的产物会触发越界 panic。现已改为切片前先验证长度、
并用 `checked_add` 防长度溢出。

## 重点回归项

- `content_key_and_plaintext_never_in_file` — 产物字节中不出现 content_key 明文或原始明文
- `register_failure_leaves_no_dead_file` — 注册失败时删除产物，不留死文件
- `test_aes256_gcm_detects_tampering` — 翻转密文比特或篡改 tag 后解密必须失败
- `test_parser_never_panics_on_truncated_input` — 产物头部被截断或长度字段被伪造时只能报错，不能 panic
- `test_register_rejects_overwrite_by_other_secret` — 换 `secret` 重新注册不得改变已登记的内容密钥
- `test_requests_and_logs_are_app_scoped` — 一个文件的密钥看不到另一个文件的申请与日志
- `blackbox_exe_self_extract_roundtrip` — 黑盒 EXE 能自解出内嵌 `.secunzip`

## 手工验证

完整的可照敲流程（含实际输出）见 [docs/demo.md](docs/demo.md)。最短版本：

启动服务端后：

```
secunzip pack ./src -o t.secunzip --server http://127.0.0.1:8090 --allow-temp
secunzip grant t.secunzip -u alice
secunzip open  t.secunzip -u alice -o ./out      # 内容应与 ./src 一致
```

服务端鉴权与审批的各项组合已由 `server/tests/api_test.rs` 自动覆盖，手工只需验证跨机器、
跨进程的真实部署：

| 用例 | 预期 |
|------|------|
| 换一台机器（非 localhost）打包后打开 | 能取到密钥并解密 |
| 服务端重启后，客户端仍能打开先前打包的文件 | 能 |
| 客户端断网时打开 | 明确报错，不静默失败 |
| 卸载后重装 | 文件关联与快捷方式恢复正常 |

## 已验证

2026-09-28：核心库与 CLI 52 个、服务端 21 个测试全部通过；本机服务端 + CLI 全链路
（打包 → 授权 → 打开）通过，产物头部算法字节为 `04`（AES-256-GCM），中文内容往返一致。

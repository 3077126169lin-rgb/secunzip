# 测试

## 运行

```
cargo test                      # 根 crate，130 个（库 57 + 集成 73）
cd server && cargo test         # 服务端，50 个（10 个单元 + 40 个集成，真实拉起进程打 HTTP 接口）
cd gui    && cargo test         # GUI，4 个（config 持久化与记住口令查找，纯逻辑）
```

三个 crate 相加：130 + 4 + 50 = **184 个**。其中根 crate 的 130 个是
库 57 + 集成 73，集成再按 4 + 16 + 3 + 20 + 3 + 10 + 6 + 4 + 3 + 4 逐文件相加得到，读者可自行核对。

CI（[.github/workflows/ci.yml](.github/workflows/ci.yml)）在每次推送与 PR 上跑这几条，外加
`cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings`。

## 分布

| 位置 | 数量 | 覆盖 |
|------|------|------|
| `src/**`（`#[cfg(test)]`） | 57 | 密钥派生与 CIDR 匹配、解压体积上限、条目路径安全、VFS、WebDAV 挂载与盘符选择、自解压识别 |
| [tests/algorithm_test.rs](tests/algorithm_test.rs) | 4 | 未实现的压缩算法必须报错而非静默替换、Zip/Store 往返、未定义的密钥派生变换必须报错 |
| [tests/cli_auth_test.rs](tests/cli_auth_test.rs) | 16 | 各子命令的 `--password` / `--remember` 参数解析、口令解析优先级（`--password` > `SECUNZIP_PASSWORD` > 配置记住的口令 > 标准输入）、申请天数只接受正整数、grant/request 报文带明文口令、approve 缺省不下发口令字段、失败时退出码非 0、记住的口令只在成功后写入 |
| [tests/cli_blackbox_test.rs](tests/cli_blackbox_test.rs) | 3 | `pack --blackbox` 参数解析、CLI 黑盒 EXE 的尾部标记与内嵌产物、普通产物不被误判 |
| [tests/crypto_test.rs](tests/crypto_test.rs) | 20 | AES-CBC/GCM 往返、ChaCha20、XChaCha20（含完整 24 字节 nonce 与长度校验）、错密钥、GCM 篡改检测、哈希、KDF、未实现算法报错 |
| [tests/header_version_test.rs](tests/header_version_test.rs) | 3 | 头部 `FORMAT_VERSION` 与当前构建不一致时必须报错：当前版本可解析、未来版本与更旧版本都被拒 |
| [tests/key_derive_test.rs](tests/key_derive_test.rs) | 10 | 派生流程节点求值、条件分支、确定性 |
| [tests/packer_test.rs](tests/packer_test.rs) | 6 | 打包/解包往返、有效期、ChaCha20、错密钥、服务端密钥解包、info |
| [tests/security_test.rs](tests/security_test.rs) | 3 | content_key 不落文件、注册失败不留死文件、黑盒 EXE 自解压 |
| [tests/robustness_test.rs](tests/robustness_test.rs) | 4 | 所有截断长度、采样位翻转、随机与边界填充输入都不得 panic |
| [tests/zip_slip_test.rs](tests/zip_slip_test.rs) | 4 | 含 `../` 或绝对路径条目的恶意 ZIP 必须解压失败并点名条目，且目标目录之外不产生文件 |
| [server/src/main.rs](server/src/main.rs)（`#[cfg(test)]`） | 10 | IP 白名单条目匹配与 CIDR 解析、有效期字符串解析、口令哈希与恒定时间比较、PBKDF2 迭代次数固定为 100000 |
| [server/tests/api_test.rs](server/tests/api_test.rs) | 40 | 服务端鉴权、注册防覆盖、多应用隔离、IP 白名单、迁移与审计保留、备份、启动失败路径 |

`src/**` 的 57 个：

- `key_derive::engine`（2）：同一流程密钥确定；不同流程密钥不同
- `key_derive::sources`（4）：CIDR 前缀匹配（/0、/32、中间前缀）与非法输入拒绝
- `packer::compress`（4）：解压体积上限（单条目与累计）被拒、限额内往返、体积可读格式化
- `runtime::loader`（4）：条目路径安全（拒绝各种越界形式）、运行目录唯一且可持久、清理不存在的基目录为空操作
- `runtime::vfs`（10）：构建、列目录、子目录、深层嵌套、读文件、读缺失、目录当文件读、文本/图片判定、总大小、空 VFS
- `runtime::mount`（31）：WebDAV OPTIONS/PROPFIND/GET/404、路径规范化、盘符选择（不返回系统盘、向下搜索、忽略软驱盘符）、卸载安全
- `runtime::selfextract`（2）：识别并解出内嵌产物、拒绝非自解压文件

## 服务端测试怎么写的

[server/tests/api_test.rs](server/tests/api_test.rs) 用 `CARGO_BIN_EXE_secunzip-server` 拉起真实的
`secunzip-server` 进程，每个用例独占一个临时目录和一个空闲端口，请求用 `std::net::TcpStream`
手写 HTTP/1.1 发出，不引入 HTTP 客户端依赖。断言既看 HTTP 响应，也直接查 SQLite 里的落库结果。

覆盖到的关键性质：

- 注册防覆盖：`app_id` 等于文件 MD5、公开可推，但猜不到 `secret` 时注册被拒，且原 `content_key` 不被改写
- 鉴权：错 `secret` 无法 grant / revoke，也看不到 requests 与 logs
- 口令鉴权：口令以哈希落库、取钥时必须匹配；口令缺失或错误的请求被拒；`approve` 不带口令时沿用申请者设定的口令；引入口令之前创建的授权被拒并要求重新授权
- 来源 IP 白名单：白名单命中放行、未命中拒绝且写 `success=false` 的审计、注册时拒绝非法与 IPv6 条目、缺省与空数组不限制
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
- `test_xchacha20_nonce_padding_mapping` — 12 字节派生 IV 映射到 24 字节 XNonce 的方式固定（前半原样、后半补零），磁盘格式不变
- `test_xchacha20_tampering_detected_by_integrity_hash` — XChaCha20 无认证标签，篡改必须由头部 SHA-256 完整性校验兜住
- `test_sm4_cbc_returns_error_not_panic` / `test_sm3_returns_error_not_panic` — 未实现的算法返回「尚未实现」错误，不得 panic
- `test_unimplemented_compress_algos_return_error` / `test_unimplemented_compress_algo_fails_pack` — 选 7z / tar.zst / tar.gz 打包必须报错并留下无产物，不得静默产出 ZIP
- `test_key_transform_concat_is_undefined` — `KeyTransform::Concat` 必须报错，不得原样返回冒充变换成功
- `test_parser_never_panics_on_truncated_input` — 产物头部被截断或长度字段被伪造时只能报错，不能 panic
- `test_register_rejects_overwrite_by_other_secret` — 换 `secret` 重新注册不得改变已登记的内容密钥
- `test_requests_and_logs_are_app_scoped` — 一个文件的密钥看不到另一个文件的申请与日志
- `test_ip_whitelist_blocks_non_matching_source` — 来源地址不在白名单时 `/api/key` 拒绝下发密钥
- `test_register_rejects_malformed_ip_whitelist` — 非法或 IPv6 白名单条目在注册时即被拒绝，不入库
- `blackbox_exe_self_extract_roundtrip` — 黑盒 EXE 能自解出内嵌 `.secunzip`
- `cli_blackbox_exe_embeds_detectable_secunzip` — CLI 的 `--blackbox` 配置产出的 EXE 带尾部标记、内嵌标准 `.secunzip`，且文件 ID 口径与 `pack_file` 一致
- `cli_secunzip_artifact_is_not_detected_as_blackbox` — 未加 `--blackbox` 的普通产物不得被误判为黑盒
- `failed_grant_exits_nonzero` — 口令缺失或被服务端拒绝时 CLI 必须非 0 退出（旧行为是打印错误仍退出 0）
- `password_precedence_arg_env_config_stdin` — 口令解析顺序固定为 `--password` > `SECUNZIP_PASSWORD` > 配置记住的口令 > 标准输入
- `approve_omits_password_when_admin_gives_none` — 管理员不填口令时请求体里连 `password` 字段都不出现，由服务端沿用申请者的口令
- `open_remembers_password_and_reuses_it` / `remembered_password_rejected_prints_actionable_hint` — `--remember` 只在成功后写入并可被后续 `open` 复用；记住的口令被拒时打印可操作的下一步
- `request_rejects_non_positive_days_before_any_io` — `--days` 非正数在本地即被拒绝，不发出任何请求
- `extract_to_dir_rejects_zip_slip_entry` / `extract_with_key_rejects_absolute_entry` — 含 `../` 或绝对路径条目时解压失败并点名条目，目标目录之外不落文件
- `future_version_is_rejected_with_both_numbers` — 头部版本高于当前构建时必须报错并同时给出两个版本号，不得按当前规则硬解

## 手工验证

完整的可照敲流程（含实际输出）见 [docs/demo.md](docs/demo.md)。最短版本：

启动服务端后：

```
secunzip pack  ./src -o t.secunzip --server http://127.0.0.1:8090 --allow-temp
secunzip grant t.secunzip -u alice --password 'alice-pw-1'
secunzip open  t.secunzip -u alice --password 'alice-pw-1' -o ./out   # 内容应与 ./src 一致
```

`grant` 与 `open` 都必须提供口令（`--password`、`SECUNZIP_PASSWORD` 或标准输入一行）；
`grant` 时设定的口令就是对方 `open` 时用的口令。

服务端鉴权与审批的各项组合已由 `server/tests/api_test.rs` 自动覆盖，手工只需验证跨机器、
跨进程的真实部署：

| 用例 | 预期 |
|------|------|
| 换一台机器（非 localhost）打包后打开 | 能取到密钥并解密 |
| 服务端重启后，客户端仍能打开先前打包的文件 | 能 |
| 客户端断网时打开 | 明确报错，不静默失败 |
| 错误口令、缺少口令、未授权用户打开 | 明确报错且退出码为 1，不静默失败 |
| 卸载后重装 | 文件关联与快捷方式恢复正常 |

## 已验证

2026-09-28：根 crate 130 个（库 57 + 集成 73）、GUI 4 个、服务端 50 个（10 个单元 + 40 个集成）
测试全部通过，合计 184 个；本机服务端 + CLI 全链路（打包 → 授权 → 打开）通过，产物头部算法字节为
`04`（AES-256-GCM），中文内容往返一致。

同日另做了一次针对口令认证的端到端验证：真实本地服务端 + 真实 CLI，覆盖正确口令取钥成功，
以及错误口令、缺少口令、未授权三种拒绝，并确认拒绝时进程退出码为 1。这是认证路径的第一次
端到端检查，此前它只有单元与集成测试覆盖。

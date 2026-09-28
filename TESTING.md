# 测试

## 运行

```
cargo test                 # 核心库 + 集成测试，48 个
cd server && cargo test
cd gui    && cargo build
```

## 分布

| 位置 | 数量 | 覆盖 |
|------|------|------|
| `src/**`（`#[cfg(test)]`） | 19 | 密钥派生引擎、VFS、WebDAV 挂载、自解压识别 |
| [tests/crypto_test.rs](tests/crypto_test.rs) | 10 | AES-CBC/GCM 往返、ChaCha20、错密钥、GCM 篡改检测、哈希、KDF |
| [tests/key_derive_test.rs](tests/key_derive_test.rs) | 10 | 派生流程节点求值、条件分支、确定性 |
| [tests/packer_test.rs](tests/packer_test.rs) | 6 | 打包/解包往返、有效期、ChaCha20、错密钥、服务端密钥解包、info |
| [tests/security_test.rs](tests/security_test.rs) | 3 | content_key 不落文件、注册失败不留死文件、黑盒 EXE 自解压 |

`src/**` 的 19 个：

- `key_derive::engine`（2）：同一流程密钥确定；不同流程密钥不同
- `runtime::vfs`（10）：构建、列目录、子目录、深层嵌套、读文件、读缺失、目录当文件读、文本/图片判定、总大小、空 VFS
- `runtime::mount`（5）：WebDAV OPTIONS/PROPFIND/GET/404、路径规范化
- `runtime::selfextract`（2）：识别并解出内嵌产物、拒绝非自解压文件

## 重点回归项

- `content_key_and_plaintext_never_in_file` — 产物字节中不出现 content_key 明文或原始明文
- `register_failure_leaves_no_dead_file` — 注册失败时删除产物，不留死文件
- `test_aes256_gcm_detects_tampering` — 翻转密文比特或篡改 tag 后解密必须失败
- `blackbox_exe_self_extract_roundtrip` — 黑盒 EXE 能自解出内嵌 `.secunzip`

## 手工验证

启动服务端后：

```
secunzip pack ./src -o t.secunzip --server http://127.0.0.1:8090 --allow-temp
secunzip grant t.secunzip -u alice
secunzip open  t.secunzip -u alice -o ./out      # 内容应与 ./src 一致
```

服务端鉴权回归（改服务端后建议逐项跑）：

| 用例 | 预期 |
|------|------|
| register 已存在 app_id + 不同 secret | 失败 |
| register 已存在 app_id + 相同 secret | 成功 |
| grant / revoke / approve / deny 错 secret | 失败 |
| requests / logs 错 secret | 失败 |
| 用 A 文件的 secret 查 B 文件的 requests | 失败 |
| requests 正确 secret | 只返回本文件的申请 |
| 未授权 user_id 取 key | 失败 |
| 已授权 user_id 取 key | 返回 content_key |

可直接用 curl 打接口复现，见 [API.md](API.md)。

## 已验证

2026-09-28：48 个测试通过；本机服务端 + CLI 全链路（打包 → 授权 → 打开）通过，
产物头部算法字节为 `04`（AES-256-GCM），中文内容往返一致；上表鉴权项逐项通过。

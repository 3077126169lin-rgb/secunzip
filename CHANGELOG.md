# 更新日志

本文件记录 SecUnzip 各版本的变更。格式遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## 未发布

### 新增

- **取密钥的身份认证**。此前 `/api/key` 只校验 `(app_id, user_id)` 是否存在授权行，而 `user_id` 就是请求体里的
  一个字符串——知道某个已授权用户 ID 的人可以直接取走密钥。这是「受控分发」这个目标上最大的逻辑缺口。
  现在每份授权带一个用户口令：PBKDF2-HMAC-SHA256、10 万轮、每行独立随机盐，比较走恒定时间实现；
  管理员路径仍用该文件的 `secret`（`secret` 本身即凭据）。
- CLI 与 GUI 的口令支持：`--password` / `SECUNZIP_PASSWORD` / 配置里记住的口令 / stdin 逐级回退；
  成功后可用 `--remember` 记住，GUI 有「记住口令」勾选框与设置页的清除入口。口令明文存于本机 `config.json`
  （该文件本就存放 `user_id` 与服务器地址），代码注释如实注明，不假装是加密存储。

### 变更

- **版本号与 tag 长期脱节**：三份 `Cargo.toml` 与 `installer.iss` 一直是 `0.1.0`，而 tag 已到 `v0.1.3`，
  于是已发布的 CLI 至今自报 `secunzip 0.1.0`。现统一到 `0.1.4`，并由发布流程强制校验 tag 与版本号一致——
  这一条原本就在发布前检查清单里，四个版本没有一个执行过，因此改为机器校验而不是靠人记得。
- **XChaCha20 改用完整的 24 字节 nonce**：此前 `iv_len()` 返回 12、后 12 字节补零，等于放弃了扩展 nonce 的全部意义。
  该算法未进过任何发布版本，因此没有兼容包袱。
- 删除死代码 `src/runtime/executor.rs`（零调用点，且含与下方同类的一处临时目录竞态）。

### 修复

- **WebDAV 挂载把明文暴露给本机所有进程**：挂载期间内容以**无认证** HTTP 服务在回环地址上，且不校验 `Host` 头，
  恶意网页可借 DNS rebinding（把域名解析回 `127.0.0.1`）从浏览器上下文读走全部明文——这直接让「不落盘、防复制」
  的卖点失效。现在每次挂载生成随机 token、请求路径必须带该 token、且校验 `Host` 必须是 `127.0.0.1:<端口>`
  或 `localhost:<端口>`；同时给手写 HTTP 解析器补上 `Content-Length` 上限、读超时、显式拒绝 `..` 与 NUL 路径段，
  `PROPFIND` 对不存在的路径返回 404（原为 207）。
- **Zip Slip**：解压时条目名未经校验就拼进目标路径，特制产物可写到用户选定目录之外。现在拒绝绝对路径、
  盘符前缀、`..` 段与 NUL 字节，并返回可读错误而不是静默跳过。
- **`FORMAT_VERSION` 从未参与判定**：更高版本写的产物会被当作当前版本解析。现在版本高于本构建时返回明确错误。
- **打开端无内存上限**：解压把每个条目全量读入内存且无上限，zip bomb 可打爆内存（打包侧早有 `MAX_CONTENT_SIZE`）。
  现在解压同样受该上限约束：读前用条目元数据预判、跨条目累计、并用 `take(remaining+1)` 防伪造元数据。
- **临时目录被提前删除**：执行 EXE 的路径写完临时目录、拉起程序后只等 1 秒就删目录，会把正在启动的程序删掉。
  改为「下次运行回收」：运行目录不再立即删除，启动时清理超过 24 小时的旧目录。
- **客户端 CIDR 在 `/0` 时移位溢出**：`1u32 << (32 - 0)` 在 debug 直接 panic，release 下按 mod 32 计算会得到
  「恰好匹配全部」的错误语义（服务端那份是对的，两处实现不一致）。现在显式处理 prefix 0。
- **CLI 出错仍返回成功**：`grant` / `revoke` / `request` / `requests` / `approve` / `deny` 打印错误后照样返回 `Ok(())`，
  退出码恒为 0；更深一层是它们完全不看服务端的 `success` 字段，`{"success":false}` 也被当成成功。
  现在失败一律上行，进程以退出码 1 结束。
- **CBC 的 padding oracle 面**：填充错误与完整性校验失败返回不同错误，构成判别信号。现在两者返回同一变体、
  同一文案，并在代码里注明「请勿为了错误信息更精确而改回」。
- **服务端数据库错误变成连接中断**：handler 里的 SQL 大量 `.unwrap()`，数据库抖动即 panic。现在开启
  SQLite 的 WAL 与 `busy_timeout`（消掉一大类瞬时锁错误），并把错误映射为 JSON 500。
- **授权与时效校验缺口**：`approve` 对不存在的申请也照样授权（现已要求存在待审批申请，且不允许产生无口令的
  授权行）；`request` 的 `need_days` 可为负数（现要求正整数）；`parse_expire` 对任意字符串原样入库
  （现只接受 `Nd` / `YYYYMMDD` / 永久）。
- **CLI 的 `approve` 会静默覆盖申请者的口令**：它把「记住的管理口令」当作可选覆盖值自动下发，于是管理员说过
  `grant --remember` 之后再审批，就会把申请者自己设的口令改掉。现在 `approve` 只认本次显式输入，留空即沿用申请口令。

## 0.1.3 - 2026-09-28

### 新增

- **XChaCha20 加密**：此前头部枚举里有该变体但派发处是 `unimplemented!()`，现在用已有的 `chacha20` crate 实现。
  派生的 12 字节 IV 原样放到 24 字节 XNonce 的前半、后半补零，`iv_len()` 保持 12，**磁盘格式不变**。
- **CLI 黑盒自解压 EXE**：`pack` 新增 `--blackbox`，产物 runner 就是打包进程自身的二进制，
  启动时先只读末尾 16 字节自查尾部标记，命中即进入黑盒打开流程（不再经过 clap）；
  argv 里出现已知子命令时仍按普通 CLI 处理，工具自身功能不受影响。CLI 与 GUI 现在都能产出、也都能打开黑盒 EXE。
- **来源 IP 白名单服务端强制**：`/api/register` 接受可选 `ip_whitelist`（IPv4 单机或 CIDR），
  `/api/key` 用 TCP 连接的对端地址校验，未命中拒绝下发密钥并写失败审计；管理员不豁免。
- IP 白名单单元测试 3 个、算法派发测试 `tests/algorithm_test.rs` 4 个、CLI 黑盒测试 `tests/cli_blackbox_test.rs` 3 个。

### 变更

- **测试从 82 个增至 103 个**：核心库与 CLI 73 个，服务端 30 个（3 个单元 + 27 个集成）。
- 服务端 schema 升级到 v2：`apps` 表新增 `ip_whitelist` 列，v1 旧库启动时自动迁移，旧数据保留。

### 修复

- **未实现的算法不再 panic**：`Sm4Cbc` 与 `Sm3` 由 `unimplemented!()` 改为返回明确的「尚未实现」中文错误，
  相关工厂不再写 `_ =>` 兜底，新增变体时必须显式处理。
- **不再静默替换算法**：`CompressAlgo::SevenZ` / `TarZst` / `TarGz` 此前会静默按 ZIP 打包，
  用户以为拿到 7z 实际得到 ZIP；现在直接返回错误。
- **`KeyTransform::Concat` 不再假装成功**：此前是原样返回的空操作，现在返回「变换未定义」错误；
  真正的拼接由 `KeyNode::Concat` 承担，行为不变。
- 删除从未声明为模块、从未参与编译的死代码 `src/runtime/stub.rs`，以及 `src/core/config.rs` 中无引用的 `ProjectConfig`
  （`PackHeader` 不受影响）。`assets/runtime_stub.exe` 是另一回事，仍是打包时的 11 字节占位回退，保留。

## 0.1.2 - 2026-09-28

### 变更

- 文档诚实化，**代码零改动**：README 新增「实现状态」表，逐条标注每项能力是已实现、部分实现还是未实现；
  `docs/design.md` 与 `docs/technical.md` 订正了与实现不符的描述。
- README、`CONTRIBUTING.md`、`SECURITY.md` 均声明本项目为课程作业、完成后不再维护。
- 本版本只为让安装包内附带的 `README.md` 与仓库主页一致。

## 0.1.1 - 2026-09-28

### 修复

- **盘符挂载会误删用户自己的映射**：此前固定挂载到 `Z:`，且在映射失败（该盘符已被网络共享、U 盘或 VHD 占用）
  时仍报告成功，随后「卸载」会无条件执行 `net use Z: /delete /y`，删掉用户自己的映射。
  现在自动挑选空闲盘符，`net use` 失败如实报错，且只在确实由本程序建立的映射上执行删除。
- **产物头部解析越界 panic**：`PackHeader::from_bytes` 用文件内的长度字段直接切片，
  被截断或长度字段被伪造的产物会让程序 panic 而非返回错误。

### 变更

- 三个 crate 合并为一个 Cargo workspace，共用一份 `Cargo.lock` 与 `target/`，
  公共依赖只编译一次；根目录的 `[profile.release]`（LTO、strip）因此对三个成员统一生效，
  产物明显变小。
- 移除 10 个声明了但源码零引用的依赖（核心库 4、服务端 2、图形界面 4）。
- 运行模式、挂载方式、`execute_sandbox` 等注释与文档改为如实描述实现状况，
  不再声称未实现的能力。

### 新增

- `GET /healthz` 存活探针，不访问数据库、无需鉴权。
- CI：每次推送与 PR 执行 `cargo fmt --all --check`、`cargo clippy --workspace -D warnings`
  与全部测试；发布流程增加 Linux 服务端产物（静态链接 musl）。
- 测试从 73 个增至 82 个：新增服务端黑盒集成测试 22 个（鉴权、注册防覆盖、多应用隔离、
  迁移与审计保留、备份、启动失败路径、健康检查）、解析健壮性测试 4 个、盘符选择与卸载安全测试 8 个。
- 工程文件：`.gitattributes`、`rustfmt.toml`、`.editorconfig`、`CONTRIBUTING.md`、`SECURITY.md`、
  `CODE_OF_CONDUCT.md`、`CHANGELOG.md`、issue 与 PR 模板、`DISCLAIMER.txt`（安装包首屏强制阅读）。

## 0.1.0 - 2026-09-28

首个版本。

### 新增

- 核心库与 CLI（`secunzip`）
  - 打包：`pack` 把源路径归档为 ZIP 后整体加密，产出 `xxx.secunzip` 与 `xxx.secret`
  - 打开：`open` 校验完整性并解密，默认在内存 VFS 中浏览、不落盘，`-o` 可解压到目录
  - 授权管理：`grant` / `revoke` / `request` / `requests` / `approve` / `deny`
  - `-e/--expires` 接受 `7d` 或 `YYYYMMDD`，省略为永久
- 加密与密钥派生
  - AES-256-GCM（默认，认证加密）、AES-256-CBC、ChaCha20
  - PBKDF2-HMAC-SHA256（10000 轮）派生加密密钥与 IV
  - 哈希：SHA-256 / SHA-512 / BLAKE3 / MD5；文件 ID 取产物内容的 MD5
  - 密钥派生流程引擎：按 `KeyNode` 树确定性求值，来源含机器码、本机 IP、日期、域用户、字面量与盐
- 运行时
  - 内存虚拟文件系统：目录树、读取、文本/图片判定
  - 挂载为盘符或 WebDAV 服务
  - 解压后执行：内存执行 / 临时目录 / 调用系统打开
  - 黑盒自解压 EXE 与交互式命令行存根；`--features runpe` 下的内存 EXE 执行（默认不编译）
- 服务端 `secunzip-server`（Axum + SQLite，默认监听 `0.0.0.0:8090`）
  - 9 个接口：`/api/register`、`/api/key`、`/api/grant`、`/api/revoke`、`/api/request`、
    `/api/requests`、`/api/approve`、`/api/deny`、`/api/logs`
  - `register` 防覆盖：`app_id` 已存在时只接受相同的 `secret`
  - register / grant / revoke / key / request / approve / deny 全部落审计日志，
    按每个 `app_id` 最近 N 条保留（默认 1000）
  - `PRAGMA user_version` 顺序迁移；`--backup` 用 `VACUUM INTO` 在线生成一致性快照
- 图形客户端 `secunzip-gui`（egui）：打开 / 打包 / 管理此文件 / 设置页，
  后台连通性探测与待审批自动拉取，Windows 下注册 `.secunzip` 文件关联
- 构建与发布
  - Windows 安装包（Inno Setup 6，按用户安装，不需要管理员，自带卸载程序，首屏强制阅读免责声明）
  - 推 `v*` tag 触发的发布工作流；提交与 PR 触发的 [CI](.github/workflows/ci.yml)（格式检查、clippy、全部测试）
  - 一键部署脚本（NSSM / systemd）与 Docker
- 73 个测试：`src/**` 单元测试 19、[tests/crypto_test.rs](tests/crypto_test.rs) 10、
  [tests/key_derive_test.rs](tests/key_derive_test.rs) 10、[tests/packer_test.rs](tests/packer_test.rs) 6、
  [tests/security_test.rs](tests/security_test.rs) 3、[tests/robustness_test.rs](tests/robustness_test.rs) 4、
  [server/tests/api_test.rs](server/tests/api_test.rs) 21
- 文档：本文件、[README.md](README.md)、[API.md](API.md)、[TESTING.md](TESTING.md)、
  [SECURITY.md](SECURITY.md)、[CONTRIBUTING.md](CONTRIBUTING.md)、[DISCLAIMER.txt](DISCLAIMER.txt)，
  以及 [docs/](docs/) 下的设计文档、技术方案清单、演示脚本与发布说明

### 安全

- 产物头部解析此前会按文件内的长度字段直接切片，遇到被截断或伪造 length 的产物会 panic；
  现在所有切片前都先校验长度并用 `checked_add` 防溢出，非法输入一律返回错误
- 这是学生项目，未做安全审计与渗透测试；README、[SECURITY.md](SECURITY.md) 与安装包首屏均声明该点，
  并建议仅在沙盒环境试用
- `content_key` 不写入产物，仅由服务端托管；Remote 模式下文件头中的 `key_derive` 字段被清空
- 已知且接受的风险（明文 HTTP、MD5 文件 ID、无速率限制、非恒定时间 secret 比较、文件头无 MAC）
  见 [SECURITY.md](SECURITY.md)

---

版本号遵循[语义化版本](https://semver.org/lang/zh-CN/)。

# 演示脚本

一套可以照着敲、当场跑通的演示流程。所有输出都是实际运行抓下来的。

## 准备

```bash
# 构建（一次即可）
cargo build --release
cd server && cargo build --release && cd ..
```

开两个终端。

**终端 1 — 服务端**

```bash
cd server
./target/release/secunzip-server
# 输出：SecUnzip Server on http://0.0.0.0:8090
```

数据库是工作目录下的 `secunzip.db`，首次运行自动建表。
换端口：`SECUNZIP_PORT=9000 ./target/release/secunzip-server`。

**终端 2 — 客户端**

```bash
cd /tmp && mkdir -p demo/src && echo hello > demo/src/readme.txt && cd demo
alias sz=/path/to/target/release/secunzip    # 或直接用完整路径
```

---

## 演示一：最简链路（约 1 分钟）

**1. 打包并注册到服务端**

```bash
sz pack ./src -o demo.secunzip --server http://127.0.0.1:8090 --allow-temp
```

```
管理信息（请妥善保管）:
文件ID:    2c9950ed9c5a599520bc98a489340a96  (打包文件的 MD5)
管理密钥:  3299be48-2df2-4dbc-a355-d42849a029cd
服务端:    http://127.0.0.1:8090
临时申请:   已开启

授权用户:
secunzip grant <文件> --user <用户ID> [--expires 7d]

用户申请:
secunzip request <文件> --user <用户ID> --days 3

查看申请:
secunzip requests <文件>
```

产出两个文件：`demo.secunzip`（可对外分发）和 `demo.secret`（管理密钥，**不要发出去**）。

**2. 授权一个用户**

```bash
sz grant demo.secunzip -u alice@example.com
```

```
授权用户 alice@example.com...
已授权 alice@example.com
```

**3. 该用户打开**

```bash
sz open demo.secunzip -u alice@example.com -o ./out
```

```
联网验证...
用户: alice@example.com
验证通过，获取到解密密钥
解压完成: /tmp/demo/out
```

**4. 验证内容一致**

```bash
diff ./src/readme.txt ./out/readme.txt && echo "内容一致"
```

**5. 未授权用户打不开**

```bash
sz open demo.secunzip -u bob@example.com -o ./out2
```

```
联网验证...
用户: bob@example.com
未授权，请申请临时权限或联系管理员
提示: 如果没有权限，可以申请临时权限:
```

> 这一步是演示的重点：**文件就在本地，但少了服务端那一次授权，就是打不开。**

---

## 演示二：临时申请与审批（约 1 分钟）

**1. 用户自助申请**（需要打包时带 `--allow-temp`）

```bash
sz request demo.secunzip -u bob --days 3 --message "想看一下课件"
```

```
申请临时权限...
已提交申请，请等待管理员审批
```

**2. 管理员查看待审批**

```bash
sz requests demo.secunzip
```

```
待审批列表:

用户: bob
需要: 3天
说明: 想看一下课件
时间: 20260928025915
```

**3. 管理员批准**

```bash
sz approve demo.secunzip -u bob
```

```
审批通过 bob...
已通过 bob
```

**4. bob 这次能打开了**

```bash
sz open demo.secunzip -u bob -o ./outb
# 验证通过，获取到解密密钥
```

**5. 吊销授权**

```bash
sz revoke demo.secunzip -u bob

sz open demo.secunzip -u bob -o ./outb2
```

```
联网验证...
用户: bob
未授权，请申请临时权限或联系管理员
```

> 讲的时候要补一句边界：**吊销只挡住"以后再取密钥"**。如果 bob 在吊销前已经打开过并自行保存了密钥，那份内容他仍能解开——详见 [设计文档 §7](design.md)。

---

## 演示三：图形客户端（约 30 秒）

```bash
cd gui && cargo run
```

| 页签 | 演示动作 |
|------|---------|
| 打开 | 未加载文件时列出**我打包的文件**；点「打开」加载内容；本人可见「管理此文件」 |
| 打包 | 选源目录 → 选输出 → 「开始打包」 |
| 设置 | 填个人 ID 与服务端地址，点「测试连接」看顶栏状态灯变绿 |

界面的三个页签都值得点一遍——评审主要看这里。

---

## 演示中可能出的问题

| 现象 | 原因 / 处理 |
|------|------------|
| `注册服务端失败，已清理打包产物` | 服务端没起来或地址不对。程序**故意删掉了产物**，避免留下打不开的死文件。先 `curl http://127.0.0.1:8090/` 确认返回 404（服务活着） |
| 对方提示 `未授权` | 少了 `grant`，或对方没 `request` + 你 `approve`。注意打包时要带 `--allow-temp` |
| `未找到密钥文件 xxx.secret` | 管理端命令需要 `.secret`；把它放回产物同目录，或 `export SECUNZIP_SECRET=<密钥>` |
| 换端口后打不开 | 打包时 `--server` 写的是哪个端口，客户端就连哪个；服务端换端口后旧产物仍指向旧地址 |
| 端口被占 | `SECUNZIP_PORT=9000 ./secunzip-server`，重新打包时 `--server` 一起改 |

---

## 答辩时可以这样讲

1. **要解决什么**：文件发出去就收不回来。目标是让"顺手转发"不再有效——接收方必须联网、且被授权才能打开。
2. **怎么做到的**：内容先用 ZIP 归档，再整体加密；**加密密钥不写在文件里**，只托管在服务端，打开时联网换一次。文件 ID 是产物内容的 MD5（内容寻址），服务端用它登记授权。
3. **为什么客户端拿不到密钥就没用**：`.secunzip` 里只有密文和明文头部，头部里的 `key_derive` 字段在联网模式下被清空——有单测专门断言"密钥与明文绝不出现在产物字节里"（`tests/security_test.rs`）。
4. **安全边界（这条一定要主动说）**：传输是明文 HTTP、客户端必然拿到密钥所以"授权一次≈永久可解密"。项目定位是**防顺手转发，不防有心破解**——[设计文档](design.md) 里写清楚了。
5. **工程量**：核心库 + CLI + 服务端 + GUI，48 个自动化测试，服务端含鉴权、审计、顺序迁移与在线备份。

# SecUnzip 完整流程：打包、授权与联网打开

本示例演示如何使用 SecUnzip 完成一次受控内容分发：把一份资料打包加密，授权指定用户，对方联网取得密钥后打开，并校验内容一致。文中同时给出命令行与图形客户端的对应操作，所有命令输出均为实际运行结果。

## 前置条件

- 已构建三个可执行文件（见 [README](../README.md#构建)）
- 服务端与客户端在同一网络内，且客户端能访问到服务端地址
- 一个可写的空目录作为演示工作区
- 图形客户端需要桌面环境

## 步骤

### 1. 启动服务端

1. 在终端中启动服务端：

   ```bash
   cd server
   ./target/release/secunzip-server
   ```

   输出：

   ```
   SecUnzip Server on http://0.0.0.0:8090
   ```

   数据库是工作目录下的 `secunzip.db`，首次运行自动建表。

2. 另开一个终端自检服务是否在跑：

   ```bash
   curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8090/
   ```

   返回 `404` 即表示服务正在运行（根路径没有页面，属正常）。

> 需要换端口时：`SECUNZIP_PORT=9000 ./target/release/secunzip-server`；客户端打包时 `--server` 要写对应端口。

### 2. 准备资料并打包

1. 建一个演示目录，放两个文件：

   ```bash
   mkdir -p demo/项目资料 && cd demo
   echo "这是一份演示用的资料" > 项目资料/说明.txt
   printf 'id,name\n1,alpha\n' > 项目资料/数据.csv
   ```

   仓库里已有现成样本 [testdata/](../testdata/)，想省掉这一步可以直接把下面的
   `./项目资料` 换成 `./testdata`。

2. 打包并注册到服务端（`--allow-temp` 表示允许对方自助申请临时权限）：

   ```bash
   secunzip pack ./项目资料 -o 项目资料.secunzip \
       --server http://127.0.0.1:8090 --allow-temp
   ```

   输出：

   ```
   管理信息（请妥善保管）:
   文件ID:    6b197f530efccc54aa81cf2b046bc4cb  (打包文件的 MD5)
   管理密钥:  18d9f652-681e-424a-8cf4-e1e6e8331703
   服务端:    http://127.0.0.1:8090
   临时申请:   已开启
   ```

3. 查看产物：

   ```bash
   ls -l 项目资料.secunzip 项目资料.secret
   ```

   - `项目资料.secunzip` —— 加密产物，可对外分发
   - `项目资料.secret` —— 该文件的管理密钥，**不要与产物一起发出去**

> 文件 ID 是产物内容的 MD5（内容寻址），打开时由客户端重新计算，不需要手动分配。`content_key` 只托管在服务端，不写入产物。

### 3. 授权接收方

```bash
secunzip grant 项目资料.secunzip -u demo@example.com
```

```
授权用户 demo@example.com...
已授权 demo@example.com
```

指定有效期可加 `-e`：`-e 7d` 表示 7 天后过期，`-e 20261231` 表示指定日期，省略为永久。

### 4. 接收方联网打开

1. 接收方执行：

   ```bash
   secunzip open 项目资料.secunzip -u demo@example.com -o ./out
   ```

   ```
   联网验证...
   用户: demo@example.com
   验证通过，获取到解密密钥
   解压完成: /tmp/demo/out
   ```

2. 校验内容一致：

   ```bash
   diff ./项目资料/说明.txt ./out/说明.txt && echo "内容一致"
   ```

3. **未授权的用户打不开**：

   ```bash
   secunzip open 项目资料.secunzip -u bob@example.com -o ./out2
   ```

   ```
   联网验证...
   用户: bob@example.com
   未授权，请申请临时权限或联系管理员
   提示: 如果没有权限，可以申请临时权限:
   ```

> 这一步是整条链路的重点：**文件就在本地，但少了服务端那一次授权，就是打不开。**

### 5. 临时申请与审批

1. 用户自助申请（需要打包时带 `--allow-temp`）：

   ```bash
   secunzip request 项目资料.secunzip -u bob --days 3 --message "想看一下资料"
   ```

   ```
   申请临时权限...
   已提交申请，请等待管理员审批
   ```

2. 管理员查看待审批列表：

   ```bash
   secunzip requests 项目资料.secunzip
   ```

   ```
   待审批列表:

   用户: bob
   需要: 3天
   说明: 想看一下资料
   时间: 20260928025915
   ```

3. 管理员批准：

   ```bash
   secunzip approve 项目资料.secunzip -u bob
   ```

   ```
   审批通过 bob...
   已通过 bob
   ```

4. 此时 bob 可以打开：

   ```bash
   secunzip open 项目资料.secunzip -u bob -o ./outb
   # 验证通过，获取到解密密钥
   ```

5. 管理员吊销授权后，bob 再次打开会被拒绝：

   ```bash
   secunzip revoke 项目资料.secunzip -u bob
   secunzip open   项目资料.secunzip -u bob -o ./outb2
   ```

   ```
   联网验证...
   用户: bob
   未授权，请申请临时权限或联系管理员
   ```

> 吊销的边界要讲清楚：它只挡住**今后**的取密钥。如果 bob 在吊销前已经打开过并自行保存了密钥，那份内容他仍能解开。详见 [设计文档](design.md)。

### 6. 图形客户端

命令行之外，日常使用建议用图形客户端；它与 CLI 共用同一套服务端与本地配置。

1. 启动客户端：

   ```bash
   cd gui && cargo run
   ```

2. **打开页**。未加载文件时列出本机打包过的文件，点「打开」即可加载并进入管理：

   ![我打包的文件](./images/open-mine.png)

3. **已加载文件**。载入产物后显示文件头信息；若同目录存在 `.secret`，本人即为该文件管理员，会多出「管理此文件」页签：

   ![已加载文件](./images/open-file.png)

4. **打包页**。选择源文件/文件夹与输出位置，可开启「允许他人申请临时权限」：

   ![打包页](./images/pack.png)

5. **设置页**。填写个人 ID 与服务端地址，顶栏状态灯会显示服务端连通性：

   ![设置页](./images/settings.png)

## 备注

- **安全边界**：传输为明文 HTTP，`content_key` 与 `secret` 会过网，只应部署在可信内网；客户端为解密必然拿到密钥，因此"授权一次 ≈ 永久可解密"。本项目定位是**提高顺手转发的成本，而非对抗有决心的攻击者**。
- **勿外发 `.secret`**：它等于该文件的最高权限，可授权、吊销与审批。
- **换端口**：服务端设 `SECUNZIP_PORT`；客户端在打包时用 `--server` 指定，旧产物仍指向旧地址。
- **注册失败不留死文件**：打包时若连不上服务端，程序会**自动删除**刚生成的产物并报错，避免留下打不开的文件。

### 常见问题

| 现象 | 原因 / 处理 |
|------|------------|
| `注册服务端失败，已清理打包产物` | 服务端未启动或地址不对。先 `curl http://127.0.0.1:8090/` 确认返回 404 |
| 对方提示 `未授权` | 少了 `grant`，或对方未 `request` + 你 `approve`；注意打包时要带 `--allow-temp` |
| `未找到密钥文件 xxx.secret` | 管理端命令需要 `.secret`。放回产物同目录，或设 `SECUNZIP_SECRET` 环境变量 |
| 端口被占用 | 换 `SECUNZIP_PORT` 重新启动服务端，并重新打包 |

### 可进一步验证的项

- 完整性：见 [TESTING.md](../TESTING.md) 的「重点回归项」与「服务端鉴权回归」表
- 产物不含密钥与明文：`tests/security_test.rs` 中有断言
- 服务端接口字段与鉴权：见 [API.md](../API.md)

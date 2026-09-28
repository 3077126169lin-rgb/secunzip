# SecUnzip 服务端接口

Axum + SQLite，默认监听 `0.0.0.0:8090`（`SECUNZIP_PORT` 可覆盖）。
服务端注册 `GET /healthz`（存活探针）与 9 个 `POST` + JSON 接口，未注册 `GET /`。
客户端的连通性探测请求根路径 `GET /`，只要收到任意 HTTP 响应（包括 Axum 对未注册路径返回的 404）即判定在线。

## 约定

响应体统一为：

```json
{"success": true, "message": "成功", "key": "...", "requests": [...]}
```

`key` 仅 `/api/key` 返回，`requests` 仅 `/api/requests` 返回。
业务失败时 `success=false`、`message` 为原因，HTTP 状态码仍为 200；请求体缺必填字段返回 422。
例外：`password`（grant / request / key）与 `need_days`（request）缺失时按业务失败处理，
返回 `success=false` 与中文原因，而不是 422。

服务端自身故障（目前只有数据库读写出错）返回 **HTTP 500**，响应体仍与上面同形：

```json
{"success":false,"message":"服务器内部错误：数据库操作失败，请稍后重试"}
```

`message` 是固定中文文案，不包含 SQL、表名或库文件路径；500 与业务失败（200 + `success=false`）
的区别在于前者表示「服务端坏了，重试可能有用」，后者表示「请求被业务规则拒绝」。
调用方应同时判断状态码与 `success` 字段，不要把 500 当成普通业务失败来提示用户。

`app_id` = 打包产物内容的 MD5，不写入文件头，客户端打开时重算。
`content_key` 只存在服务端，不写入产物。

鉴权凭据三类：`secret`（管理密钥，打包时生成）、`user_id`（用户标识）与 `password`（每个用户的取钥口令）。
`user_id` 只是请求体里的字符串，单独用它取钥等于谁填对被授权人标识谁就能拿密钥；
因此普通用户路径必须同时提供 `user_id` 与 `password`，只有管理员 `secret` 路径免口令。

口令散列：PBKDF2-HMAC-SHA256，100000 次迭代；每个用户一份 16 字节随机盐（小写 hex），
存库的是 32 字节派生结果的小写 hex（`pwd_salt` / `pwd_hash`），不存明文。
比对为恒定时间比较。同一口令在不同用户/不同次授权下的散列不同（盐不同）。

传输为明文 HTTP，`content_key`、`secret` 与 `password` 会过网，只应部署在可信内网。
见 [docs/design.md](docs/design.md)。

## 接口

### GET /healthz

存活探针，供容器编排 / 端口转发器使用。无需鉴权，无请求体，不访问数据库，固定返回 200。

```json
{"status":"ok"}
```

### POST /api/register

打包者注册并托管 content_key。若 `app_id` 已存在，`secret` 必须与已登记的一致，否则拒绝。

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| app_id | string | 是 | 文件ID（MD5） |
| secret | string | 是 | 管理密钥 |
| content_key | string | 是 | 文件加密密钥 |
| allow_temp | bool | 否 | 允许用户自助申请临时权限，默认 false |
| ip_whitelist | string[] | 否 | 允许取钥的来源 IP，IPv4 单机或 CIDR；缺省 / 空数组为不限制 |

```json
{"success":true,"message":"应用已注册"}
```

同 `app_id` + 同 `secret` 的重复注册是幂等的，会更新 `content_key` 与 `allow_temp`。

`ip_whitelist` 在注册时校验：条目只能是 IPv4 单机（`203.0.113.7`）或 IPv4 CIDR（`203.0.113.0/24`，前缀 0–32），
IPv6 条目与非法写法一律拒绝注册（`IP 白名单条目非法（仅支持 IPv4 单机或 CIDR…）`），且不写入数据库。
入库形式为非空条目组成的 JSON 数组；省略字段、传空数组都存空值，含义相同：不限制来源 IP。
重复注册视为对全部字段的完整声明，因此同 `app_id` + 同 `secret` 再次注册而省略该字段会清除已有白名单。

### POST /api/key

取解密密钥。传 `secret` 为管理员直取（`secret` 本身就是凭据，不需要口令）；
否则必须同时传 `user_id` 与 `password`，且存在未过期、已设置口令的授权。

| 字段 | 类型 | 必填 |
|------|------|------|
| app_id | string | 是 |
| secret | string | 否 |
| user_id | string | 普通用户必填 |
| password | string | 普通用户必填 |

```json
{"success":true,"message":"成功","key":"<content_key>"}
```

失败：`未授权，请申请临时权限或联系管理员`；`权限已过期（YYYYMMDD）`（过期授权会被删除）；
`该授权未设置口令，请管理员重新授权`；`未提供口令：请填写授权时设置的口令`；`口令错误`；
`来源 IP <ip> 不在该文件的 IP 白名单内，拒绝下发密钥`。

`password` 缺失、为空、或与该授权登记的口令不符都不下发密钥（失败响应不含 `key` 字段），
并各写一条 `action=key`、`success=false` 的审计记录，`details` 写明原因（`未提供口令` / `口令错误` /
`该授权未设置口令`）。口令比对是恒定时间的，不因前若干字节相同而提前返回。

`pwd_hash` 为 NULL 的授权（本次改动前建立的授权，或由迁移从旧库带过来的授权）
一律拒绝：库里没有口令散列就无法证明取钥人是被授权人。管理员用 `/api/grant`
对该用户重新授权（带 `password`）即可恢复。

该文件的 `ip_whitelist` 非空时，服务端按连接的对端地址做匹配（不接受请求体里自报的 IP）：
命中任一条目才下发密钥，管理员 `secret` 路径同样受限；未命中时不返回 `key`，
并写一条 `action=key`、`success=false` 的审计记录（`details` 写明来源 IP 与原因）。
IP 白名单校验先于口令校验执行。
匹配只实现 IPv4：IPv6 条目不参与匹配（注册时即被拒绝），IPv6 来源地址在白名单非空时一律拒绝。
经反向代理 / 端口转发访问时服务端看到的是代理地址，见 [docs/design.md](docs/design.md) §7。

### POST /api/grant

授权用户。鉴权：`secret`。必须设置该用户的取钥口令，否则拒绝（不写库）。

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| app_id | string | 是 | |
| secret | string | 是 | |
| user_id | string | 是 | |
| password | string | 是 | 该用户取钥时的口令；缺失或为空一律拒绝 |
| expires_at | string | 否 | `Nd`、`YYYYMMDD`；空 / `永久` / `永久有效` / `permanent` 为永久 |

```json
{"success":true,"message":"已授权 alice"}
```

失败：`密钥错误`；`缺少口令：授权时必须为该用户设置取钥口令`；过期时间格式非法时返回
`过期时间格式非法（支持 Nd、YYYYMMDD、永久）: <输入>` 或 `过期天数必须为正整数: <输入>`。

`expires_at` 只接受以下写法，其余（含 `2025-01-01`、任意文本）一律拒绝，不会当作日期存库：

| 写法 | 含义 |
|------|------|
| 省略 / 空串 / `永久` / `永久有效` / `permanent`（ASCII 大小写不敏感） | 无过期 |
| `Nd` / `ND`，N 为正整数（如 `30d`） | N 天后过期，存为 `YYYYMMDD` |
| 恰好 8 位 ASCII 数字 `YYYYMMDD`（如 `20301231`） | 该日期过期，原样存储 |

对同一 `(app_id, user_id)` 重复授权是幂等的，会覆盖过期时间与口令（重新授权就是改口令的方式）。
无口令的授权不得存在：它等价于「知道 user_id 就能取钥」。

### POST /api/revoke

吊销用户（删除授权记录）。鉴权：`secret`。字段同 grant（`expires_at`、`password` 忽略）。

### POST /api/request

申请临时权限。无需鉴权，但该文件 `allow_temp` 必须为 true。
用户须自行设定口令，审批通过后系统按申请时的天数自动计算过期时间。

| 字段 | 类型 | 必填 |
|------|------|------|
| app_id | string | 是 |
| user_id | string | 是 |
| need_days | int | 是 |
| password | string | 是 |
| message | string | 否 |

`need_days` 必须是正整数：缺失、0 或负数一律拒绝（`申请天数必须为正整数（need_days 至少为 1）`），
不会写库。缺 `password` 或为空同样拒绝（`缺少口令：申请时必须设置口令，审批通过后用该口令取密钥`）。
口令与天数在校验通过后才会进入后续流程。

已有授权时返回 `已有权限，直接打开即可`；已有待审批时返回 `已有待审批申请，请耐心等待`。

### POST /api/requests

查看该文件的待审批列表。鉴权：`secret`。

```json
{"success":true,"message":"成功","requests":[
  {"user_id":"alice","need_days":3,"message":"...","created_at":"20260928013417"}
]}
```

### POST /api/approve

审批通过并写入授权。鉴权：`secret`。字段同 grant，但 `password` 为选填；
`expires_at` 缺省时按申请天数计算，仍无则永久。

必须存在该 `(app_id, user_id)` 的待审批申请，否则拒绝（`没有待审批的申请，拒绝审批`），
不会凭空创建授权；同一个申请也不能重复审批（首次审批后状态不再是 `pending`）。

口令来源：请求体带 `password` 时用管理员指定的口令（可覆盖用户申请时设的口令），
否则沿用申请行上已存的 `pwd_salt` / `pwd_hash`；两者都没有则拒绝
（`该申请没有口令：请在审批时用 password 指定，或让用户重新申请`）。
审批写入的授权行一定带口令散列，不存在「无口令授权」这种中间态。

### POST /api/deny

审批拒绝。鉴权：`secret`。字段同 approve。

### POST /api/logs

查看审计日志（最近 200 条，倒序）。鉴权：`secret`。

```json
{"success":true,"message":"成功","logs":[
  {"user_id":"alice","action":"key","success":true,"details":"下发密钥","created_at":"20260928013417"}
]}
```

`action` 取值：`register` / `grant` / `revoke` / `key` / `request` / `approve` / `deny`。

## 调用示例

```bash
curl -X POST http://127.0.0.1:8090/api/register \
  -H 'Content-Type: application/json' \
  -d '{"app_id":"<md5>","secret":"<secret>","content_key":"<key>","allow_temp":true}'

# 授权时必须带上给该用户的口令
curl -X POST http://127.0.0.1:8090/api/grant \
  -H 'Content-Type: application/json' \
  -d '{"app_id":"<md5>","secret":"<secret>","user_id":"alice","password":"<用户口令>"}'

# 普通用户取钥：user_id + 口令，两者缺一不可
curl -X POST http://127.0.0.1:8090/api/key \
  -H 'Content-Type: application/json' \
  -d '{"app_id":"<md5>","user_id":"alice","password":"<用户口令>"}'

# 管理员取钥：凭 secret 直取，不需要口令
curl -X POST http://127.0.0.1:8090/api/key \
  -H 'Content-Type: application/json' \
  -d '{"app_id":"<md5>","secret":"<secret>"}'
```

## 数据库

工作目录下的 `secunzip.db`（位置由 `DATABASE_URL` 覆盖，父目录会自动创建），启动时自动建表：

| 表 | 列 |
|----|----|
| `apps` | `app_id`(PK)、`secret`、`content_key`、`allow_temp`、`ip_whitelist`、`created_at` |
| `grants` | `app_id`、`user_id`、`granted_at`、`expires_at`、`pwd_salt`、`pwd_hash`，PK(`app_id`,`user_id`) |
| `requests` | `id`、`app_id`、`user_id`、`need_days`、`message`、`status`、`created_at`、`resolved_at`、`pwd_salt`、`pwd_hash` |
| `audit_logs` | `id`、`app_id`、`user_id`、`action`、`success`、`details`、`created_at` |

连接参数由 `SqliteConnectOptions` 配置并作用于整个连接池（每条连接、含重连都带上），
不是在单条连接上执行一次 `PRAGMA`：

- `journal_mode = WAL`：读不阻塞写、写不阻塞读，写锁只作用于库文件的一部分；
  默认的 `delete` 模式每次写都要独占整库，并发请求下容易报 `SQLITE_BUSY`。
  代价是库旁边会多出 `secunzip.db-wal` 与 `secunzip.db-shm` 两个文件
  （已在 `.gitignore` 中忽略，备份 / 拷贝数据库时应连同它们一起处理，
  或先停服；`--backup` 的 `VACUUM INTO` 快照不受影响）。
- `busy_timeout = 5s`：短暂写锁争用时等待而不是立刻报错；再长只会让真正的故障迟迟不返回。
- `foreign_keys` 保持 sqlx 默认的 ON；文件不存在时自动创建（含 `DATABASE_URL` 不带 `mode=rwc` 时）。

数据库故障不再让请求 panic 断连，而是按上面的约定返回 JSON 500。
审计日志写入失败只记录在服务端，不影响主流程的返回值。

`pwd_salt` 为 16 字节随机盐的小写 hex（32 字符），`pwd_hash` 为 PBKDF2-HMAC-SHA256
100000 次迭代导出 32 字节的小写 hex（64 字符）；两者都可为 NULL。

schema 版本存于 `PRAGMA user_version`，当前为 v3。启动时按版本顺序迁移，旧数据保留：

- v1 → v2：补上 `apps.ip_whitelist` 列（旧行为 NULL，即不限制来源 IP）；
- v2 → v3：补上 `grants.pwd_salt` / `grants.pwd_hash` / `requests.pwd_salt` / `requests.pwd_hash`
  四列。SQLite 没有 `ADD COLUMN IF NOT EXISTS`，服务端按 `PRAGMA table_info` 逐列判断后再加，
  因此迁移可重复执行，不会因中断重试而失败。

迁移完成时 `pwd_salt` / `pwd_hash` 对旧行都是 NULL：旧授权在 `/api/key` 上会被拒绝并提示
`该授权未设置口令，请管理员重新授权`，管理员用 `/api/grant` 带 `password` 重新授权即可；
旧申请则在 `/api/approve` 时由管理员带 `password` 指定，或让用户重新申请。

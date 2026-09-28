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
业务失败时 `success=false`、`message` 为原因，HTTP 状态码仍为 200；请求体缺字段返回 422。

`app_id` = 打包产物内容的 MD5，不写入文件头，客户端打开时重算。
`content_key` 只存在服务端，不写入产物。

鉴权凭据两类：`secret`（管理密钥，打包时生成）和 `user_id`（用户标识）。

传输为明文 HTTP，`content_key` 与 `secret` 会过网，只应部署在可信内网。
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

取解密密钥。传 `secret` 为管理员直取；传 `user_id` 需存在未过期授权。

| 字段 | 类型 | 必填 |
|------|------|------|
| app_id | string | 是 |
| secret | string | 否 |
| user_id | string | 否 |

```json
{"success":true,"message":"成功","key":"<content_key>"}
```

失败：`未授权，请申请临时权限或联系管理员`；`权限已过期（YYYYMMDD）`（过期授权会被删除）；
`来源 IP <ip> 不在该文件的 IP 白名单内，拒绝下发密钥`。

该文件的 `ip_whitelist` 非空时，服务端按连接的对端地址做匹配（不接受请求体里自报的 IP）：
命中任一条目才下发密钥，管理员 `secret` 路径同样受限；未命中时不返回 `key`，
并写一条 `action=key`、`success=false` 的审计记录（`details` 写明来源 IP 与原因）。
匹配只实现 IPv4：IPv6 条目不参与匹配（注册时即被拒绝），IPv6 来源地址在白名单非空时一律拒绝。
经反向代理 / 端口转发访问时服务端看到的是代理地址，见 [docs/design.md](docs/design.md) §7。

### POST /api/grant

授权用户。鉴权：`secret`。

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| app_id | string | 是 | |
| secret | string | 是 | |
| user_id | string | 是 | |
| expires_at | string | 否 | `Nd`、`YYYYMMDD`；空 / `永久` / `permanent` 为永久 |

```json
{"success":true,"message":"已授权 alice"}
```

### POST /api/revoke

吊销用户（删除授权记录）。鉴权：`secret`。字段同 grant（`expires_at` 忽略）。

### POST /api/request

申请临时权限。无需鉴权，但该文件 `allow_temp` 必须为 true。

| 字段 | 类型 | 必填 |
|------|------|------|
| app_id | string | 是 |
| user_id | string | 是 |
| need_days | int | 否 |
| message | string | 否 |

已有授权时返回 `已有权限，直接打开即可`；已有待审批时返回 `已有待审批申请，请耐心等待`。

### POST /api/requests

查看该文件的待审批列表。鉴权：`secret`。

```json
{"success":true,"message":"成功","requests":[
  {"user_id":"alice","need_days":3,"message":"...","created_at":"20260928013417"}
]}
```

### POST /api/approve

审批通过并写入授权。鉴权：`secret`。字段同 grant；
`expires_at` 缺省时按申请天数计算，仍无则永久。

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

curl -X POST http://127.0.0.1:8090/api/grant \
  -H 'Content-Type: application/json' \
  -d '{"app_id":"<md5>","secret":"<secret>","user_id":"alice"}'

curl -X POST http://127.0.0.1:8090/api/key \
  -H 'Content-Type: application/json' \
  -d '{"app_id":"<md5>","user_id":"alice"}'
```

## 数据库

工作目录下的 `secunzip.db`，启动时自动建表：

| 表 | 列 |
|----|----|
| `apps` | `app_id`(PK)、`secret`、`content_key`、`allow_temp`、`ip_whitelist`、`created_at` |
| `grants` | `app_id`、`user_id`、`granted_at`、`expires_at`，PK(`app_id`,`user_id`) |
| `requests` | `id`、`app_id`、`user_id`、`need_days`、`message`、`status`、`created_at`、`resolved_at` |
| `audit_logs` | `id`、`app_id`、`user_id`、`action`、`success`、`details`、`created_at` |

schema 版本存于 `PRAGMA user_version`，当前为 v2。启动时按版本顺序迁移：
v1 的既有库会自动补上 `apps.ip_whitelist` 列（旧行为 NULL，即不限制来源 IP），旧数据保留。

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

```json
{"success":true,"message":"应用已注册"}
```

同 `app_id` + 同 `secret` 的重复注册是幂等的，会更新 `content_key` 与 `allow_temp`。

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

失败：`未授权，请申请临时权限或联系管理员`；`权限已过期（YYYYMMDD）`（过期授权会被删除）。

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
| `apps` | `app_id`(PK)、`secret`、`content_key`、`allow_temp`、`created_at` |
| `grants` | `app_id`、`user_id`、`granted_at`、`expires_at`，PK(`app_id`,`user_id`) |
| `requests` | `id`、`app_id`、`user_id`、`need_days`、`message`、`status`、`created_at`、`resolved_at` |
| `audit_logs` | `id`、`app_id`、`user_id`、`action`、`success`、`details`、`created_at` |

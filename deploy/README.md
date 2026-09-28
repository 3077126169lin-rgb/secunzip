# SecUnzip 服务端部署指南

## 方式一：一键脚本部署

### Windows

```powershell
# 默认端口 8090
.\install-windows.ps1

# 自定义端口
.\install-windows.ps1 -Port 9090
```

### Linux

```bash
# 添加执行权限
chmod +x install-linux.sh

# 默认端口 8090
./install-linux.sh

# 自定义端口
./install-linux.sh 9090
```

---

## 方式二：Docker 部署

### 使用 docker-compose（推荐）

```bash
cd deploy
docker-compose up -d
```

### 手动 Docker 构建

```bash
# 构建镜像
docker build -t secunzip-server -f deploy/Dockerfile .

# 运行容器
docker run -d \
  -p 8090:8090 \
  -v secunzip-data:/data \
  secunzip-server
```

---

## 方式三：手动编译

### 环境要求

- Rust：服务端 ≥ 1.71（tokio 的 MSRV）；完整项目含 GUI 时 ≥ 1.76（egui / eframe 的 MSRV）
- C 编译器：SQLite 以 `bundled` 方式编译进二进制，需要 `cc`（Linux 用 gcc，Windows 用 MinGW）

### 编译步骤

```bash
# 进入服务端目录
cd server

# 编译
cargo build --release

# 运行
./target/release/secunzip-server
```

---

## 安装为系统服务

本节使用的 install-service.ps1 与 install-service.sh 不随仓库提供，需先运行 install-windows.ps1 或 install-linux.sh 生成后再执行。

### Windows

```powershell
# 需要 NSSM: https://nssm.cc/download
.\install-service.ps1
```

管理命令：
```powershell
nssm start SecUnzipServer
nssm stop SecUnzipServer
nssm status SecUnzipServer
```

### Linux

```bash
sudo ./install-service.sh
```

管理命令：
```bash
sudo systemctl status secunzip-server
sudo systemctl stop secunzip-server
sudo systemctl restart secunzip-server
sudo journalctl -u secunzip-server -f
```

---

## 配置

### 环境变量

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `DATABASE_URL` | SQLite 位置（`sqlite:` 形式；所在目录不存在会自动创建） | `sqlite:secunzip.db?mode=rwc` |
| `SECUNZIP_PORT` | 监听端口 | `8090` |
| `SECUNZIP_AUDIT_KEEP` | 每个文件的审计日志保留条数 | `1000` |

部署脚本与 Docker 都通过 `DATABASE_URL` 把数据库指到数据目录 / 数据卷上。

### 备份

```bash
# 生成一致性快照，可在服务端运行期间执行
secunzip-server --backup /path/to/backup.db
```

数据库是单文件 SQLite（默认 rollback journal，没有 `-wal`/`-shm` 附属文件），
停掉服务端后直接复制 `.db` 也同样有效。

表结构变更由启动时的**顺序迁移**自动完成（`PRAGMA user_version`，语句幂等），无需手工干预。

---

## 访问

部署完成后，服务端地址: `http://your-server:8090`

接口清单与请求/响应格式见 [../API.md](../API.md)。
连通性自检：`curl http://your-server:8090/` —— 返回 404 表示服务正在运行。

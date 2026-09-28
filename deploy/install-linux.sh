#!/bin/bash
# SecUnzip 服务端一键部署脚本 (Linux)
# 用法: chmod +x install-linux.sh && ./install-linux.sh

set -e

PORT=${1:-8080}
DB_PATH=${2:-"secunzip.db"}

echo "========================================"
echo "  SecUnzip 服务端部署脚本 (Linux)"
echo "========================================"
echo ""

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# 1. 检查并安装依赖
echo -e "${YELLOW}[1/5] 检查依赖...${NC}"

# 检测包管理器
if command -v apt-get &> /dev/null; then
    PKG_MANAGER="apt-get"
    INSTALL_CMD="apt-get install -y"
elif command -v yum &> /dev/null; then
    PKG_MANAGER="yum"
    INSTALL_CMD="yum install -y"
elif command -v dnf &> /dev/null; then
    PKG_MANAGER="dnf"
    INSTALL_CMD="dnf install -y"
elif command -v pacman &> /dev/null; then
    PKG_MANAGER="pacman"
    INSTALL_CMD="pacman -S --noconfirm"
else
    echo -e "${RED}不支持的包管理器，请手动安装依赖${NC}"
    exit 1
fi

# 安装基础依赖
echo "  安装编译工具..."
sudo $INSTALL_CMD gcc make curl 2>/dev/null || true

# 2. 检查并安装 Rust
echo -e "${YELLOW}[2/5] 检查 Rust...${NC}"
if ! command - rustc &> /dev/null; then
    echo "  未找到 Rust，正在安装..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi
RUST_VER=$(rustc --version)
echo -e "  Rust: ${GREEN}$RUST_VER${NC}"

# 3. 编译服务端
echo -e "${YELLOW}[3/5] 编译服务端...${NC}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_ROOT/server"

cargo build --release
echo -e "  编译完成"

# 4. 创建部署目录
echo -e "${YELLOW}[4/5] 创建部署目录...${NC}"
DEPLOY_DIR="$PROJECT_ROOT/deploy/server"
mkdir -p "$DEPLOY_DIR"
cp "$PROJECT_ROOT/target/release/secunzip-server" "$DEPLOY_DIR/"
cd "$DEPLOY_DIR"
echo -e "  部署目录: $DEPLOY_DIR"

# 5. 创建启动脚本和服务文件
echo -e "${YELLOW}[5/5] 创建启动脚本...${NC}"

# 启动脚本
cat > "$DEPLOY_DIR/start.sh" << 'EOF'
#!/bin/bash
echo "========================================"
echo "  SecUnzip 服务端"
echo "========================================"
echo ""
echo "端口: ${PORT:-8080}"
echo "数据库: ${DB_PATH:-secunzip.db}"
echo ""
echo "按 Ctrl+C 停止服务"
echo ""

export DATABASE_URL="sqlite:${DB_PATH:-secunzip.db}?mode=rwc"
./secunzip-server
EOF
chmod +x "$DEPLOY_DIR/start.sh"

# Systemd 服务文件
cat > "$DEPLOY_DIR/secunzip-server.service" << EOF
[Unit]
Description=SecUnzip 授权服务
After=network.target

[Service]
Type=simple
User=$USER
WorkingDirectory=$DEPLOY_DIR
Environment=DATABASE_URL=sqlite:$DB_PATH?mode=rwc
ExecStart=$DEPLOY_DIR/secunzip-server
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

# 安装服务脚本
cat > "$DEPLOY_DIR/install-service.sh" << EOF
#!/bin/bash
echo "安装 systemd 服务..."

sudo cp "$DEPLOY_DIR/secunzip-server.service" /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable secunzip-server
sudo systemctl start secunzip-server

echo ""
echo "服务已安装并启动"
echo ""
echo "管理命令:"
echo "  sudo systemctl status secunzip-server   # 查看状态"
echo "  sudo systemctl stop secunzip-server      # 停止"
echo "  sudo systemctl restart secunzip-server   # 重启"
echo "  sudo journalctl -u secunzip-server -f    # 查看日志"
EOF
chmod +x "$DEPLOY_DIR/install-service.sh"

echo ""
echo -e "${GREEN}========================================"
echo "  部署完成!"
echo -e "========================================${NC}"
echo ""
echo -e "${CYAN}启动方式:${NC}"
echo "  方式1: cd $DEPLOY_DIR && ./start.sh"
echo "  方式2: cd $DEPLOY_DIR && cargo run --release"
echo ""
echo -e "${CYAN}安装为系统服务:${NC}"
echo "  sudo ./install-service.sh"
echo ""
echo -e "${CYAN}服务端地址:${NC} http://0.0.0.0:$PORT"
echo ""

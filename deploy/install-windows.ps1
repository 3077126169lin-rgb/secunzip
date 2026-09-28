# SecUnzip 服务端一键部署脚本 (Windows)
# 用法: .\install-windows.ps1

param(
    [int]$Port = 8090,
    [string]$DbPath = "secunzip.db"
)

$ErrorActionPreference = "Stop"

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  SecUnzip 服务端部署脚本 (Windows)" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""

# 1. 检查 Rust
Write-Host "[1/5] 检查 Rust..." -ForegroundColor Yellow
if (!(Get-Command rustc -ErrorAction SilentlyContinue)) {
    Write-Host "  未找到 Rust，正在安装..." -ForegroundColor Red
    winget install Rustlang.Rustup --accept-package-agreements --accept-source-agreements
    $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")
}
$rustVer = rustc --version
Write-Host "  Rust: $rustVer" -ForegroundColor Green

# 2. 检查/安装编译工具
Write-Host "[2/5] 检查编译工具..." -ForegroundColor Yellow
if (!(Get-Command gcc -ErrorAction SilentlyContinue)) {
    Write-Host "  未找到 MinGW，正在安装..." -ForegroundColor Red
    winget install MSYS2.MSYS2 --accept-package-agreements --accept-source-agreements
    & "C:\msys64\usr\bin\bash.exe" -lc "pacman -S --noconfirm mingw-w64-x86_64-gcc"
    $env:Path = "C:\msys64\mingw64\bin;" + $env:Path
}
Write-Host "  编译工具: OK" -ForegroundColor Green

# 3. 编译服务端
Write-Host "[3/5] 编译服务端..." -ForegroundColor Yellow
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$projectRoot = Split-Path -Parent $scriptDir
Set-Location "$projectRoot\server"

$env:Path = "C:\msys64\mingw64\bin;" + [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")

cargo build --release
if ($LASTEXITCODE -ne 0) {
    Write-Host "  编译失败!" -ForegroundColor Red
    exit 1
}
Write-Host "  编译完成" -ForegroundColor Green

# 4. 创建部署目录
Write-Host "[4/5] 创建部署目录..." -ForegroundColor Yellow
$deployDir = "$projectRoot\deploy\server"
New-Item -ItemType Directory -Path $deployDir -Force | Out-Null
Copy-Item "$projectRoot\server\target\release\secunzip-server.exe" $deployDir -Force
Set-Location $deployDir
Write-Host "  部署目录: $deployDir" -ForegroundColor Green

# 5. 创建启动脚本
Write-Host "[5/5] 创建启动脚本..." -ForegroundColor Yellow
$startScript = @"
@echo off
echo ========================================
echo   SecUnzip 服务端
echo ========================================
echo.
echo 端口: $Port
echo 数据库: $DbPath
echo.
echo 按 Ctrl+C 停止服务
echo.

set DATABASE_URL=sqlite:$DbPath?mode=rwc
set SECUNZIP_PORT=$Port
secunzip-server.exe

pause
"@
Set-Content -Path "$deployDir\start.bat" -Value $startScript

# 创建服务安装脚本（NSSM）
$nssmScript = @"
# 使用 NSSM 安装为 Windows 服务
# 需要先下载 NSSM: https://nssm.cc/download

`$nssm = "nssm.exe"
`$serviceName = "SecUnzipServer"
`$exePath = "$deployDir\secunzip-server.exe"

if (!(Get-Command `$nssm -ErrorAction SilentlyContinue)) {
    Write-Host "请先下载 NSSM: https://nssm.cc/download" -ForegroundColor Red
    Write-Host "并将 nssm.exe 放入 PATH" -ForegroundColor Red
    exit 1
}

& `$nssm install `$serviceName `$exePath
& `$nssm set `$serviceName AppDirectory `$deployDir
& `$nssm set `$serviceName DisplayName "SecUnzip 授权服务"
& `$nssm set `$serviceName Description "SecUnzip 受控内容分发授权服务"
& `$nssm set `$serviceName Start SERVICE_AUTO_START

Write-Host "服务已安装，使用以下命令管理:" -ForegroundColor Green
Write-Host "  nssm start SecUnzipServer" -ForegroundColor Cyan
Write-Host "  nssm stop SecUnzipServer" -ForegroundColor Cyan
Write-Host "  nssm status SecUnzipServer" -ForegroundColor Cyan
"@
Set-Content -Path "$deployDir\install-service.ps1" -Value $nssmScript

Write-Host ""
Write-Host "========================================" -ForegroundColor Green
Write-Host "  部署完成!" -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Green
Write-Host ""
Write-Host "启动方式:" -ForegroundColor Cyan
Write-Host "  方式1: 双击 start.bat" -ForegroundColor White
Write-Host "  方式2: cd $deployDir && cargo run --release" -ForegroundColor White
Write-Host ""
Write-Host "安装为系统服务:" -ForegroundColor Cyan
Write-Host "  .\install-service.ps1" -ForegroundColor White
Write-Host ""
Write-Host "服务端地址: http://0.0.0.0:$Port" -ForegroundColor Cyan
Write-Host ""

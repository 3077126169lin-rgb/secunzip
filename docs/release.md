# 发布与卸载

## 发布包含什么

| 产物 | 说明 |
|------|------|
| `SecUnzip-Setup.exe` | Windows 安装包（Inno Setup 6），一次装好 GUI + CLI + 服务端 |
| `secunzip.exe` | CLI，单文件，可独立使用 |
| `secunzip-server.exe` | 授权服务端，单文件，可独立部署 |

## 安装包做了什么

由 [installer.iss](../installer.iss) 定义：

1. 首屏为免责声明页（`LicenseFile=DISCLAIMER.txt`），必须点「我接受」才能继续安装
2. 把三个可执行文件装到 `{autopf}\SecUnzip` —— **按用户安装**（`PrivilegesRequired=lowest`），不需要管理员权限
3. 一并放入 `deploy/` 脚本与文档，以及 `README.md`、`ATTRIBUTION.md`、`SECURITY.md`、`DISCLAIMER.txt`、`LICENSE`
4. 创建开始菜单快捷方式（「SecUnzip 客户端」「启动服务端」「卸载 SecUnzip」「归属声明与第三方许可」「免责声明」），桌面快捷方式可选
5. 可选注册 `.secunzip` 文件关联（写 `HKCU`，不碰系统级设置）
6. 安装完成后执行一次 `secunzip-gui.exe --register` 刷新关联
7. **不安装任何依赖** —— 打包的二进制不需要 VC++ 运行库，也不依赖系统 libsqlite3
8. 图标：安装包自身、开始菜单与桌面快捷方式、以及安装目录下的 `assets\icon.ico` 共用同一套图标（由 [assets/icon.ico](../assets/icon.ico) 提供，三个 exe 内也已嵌入同一图标，见 [build.rs](../build.rs)）

## 卸载

**卸载程序随安装包一起产生，不需要单独分发。**

- Inno Setup 在安装时生成 `unins000.exe` 放进安装目录 `{app}\`
- 同时注册到 Windows「设置 → 应用 → 已安装的应用」（`CreateUninstallRegKey=yes`），显示名为 `SecUnzip`
- 开始菜单里的「卸载 SecUnzip」也可直接调用
- `[UninstallDelete]` 会一并清理安装目录下的 `secunzip.db`

三个入口任选：应用列表 / 开始菜单 / 直接运行安装目录下的 `unins000.exe`。

> `%USERPROFILE%\Documents\SecUnzip\` 下的客户端配置（`config.json`、`packed.json`）**不会被卸载程序删除** —— 那是用户数据而不是安装产物。需要彻底清理时手动删掉该目录。

## 如何产出发布件

### 方式一：推 tag 自动构建（推荐）

[.github/workflows/release.yml](../.github/workflows/release.yml) 在推送 `v*` tag 时自动：

1. 用 MinGW 工具链构建三个可执行文件（与本地构建一致，产物不依赖 VC++ 运行库）
2. 安装 Inno Setup 并执行 `installer.iss`
3. 把 `SecUnzip-Setup.exe` 与两个独立可执行文件挂到 Release 上

```bash
git tag v0.1.0
git push origin v0.1.0
```

> 该工作流**尚未实跑验证**（只有打 tag 才会触发）。首次使用时留意 Actions 日志；若 MinGW 或 Inno Setup 的安装步骤有变动，按日志调整。

### 方式二：本地手动构建

需先安装 [Inno Setup 6](https://jrsoftware.org/isinfo.php)（提供 `iscc`）。

```bash
cargo build --release
cd server && cargo build --release && cd ..
cd gui    && cargo build --release && cd ..

iscc installer.iss          # 输出 output/SecUnzip-Setup.exe
```

`installer.iss` 的 `[Files]` 读取这三个路径，构建完请确认它们存在：

```
gui\target\release\secunzip-gui.exe
target\release\secunzip.exe
server\target\release\secunzip-server.exe
```

## 发布前检查

- [ ] `cargo test` 通过（48 项）
- [ ] 三个 crate 均可 `cargo build --release`
- [ ] `installer.iss` 中的 `MyAppVersion` 与本次 tag 一致
- [ ] 实机走一遍：安装 → 启动客户端 → 打包/打开 → 从「应用」里卸载，确认 `unins000.exe` 生效

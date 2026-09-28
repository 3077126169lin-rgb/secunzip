; =============================================================
; SecUnzip 安装程序 (Inno Setup 6 脚本)
; 生成: SecUnzip-Setup.exe
; 内容: 客户端 GUI + CLI + 服务端 + 文件关联 + 快捷方式 + 卸载
; =============================================================

#define MyAppName "SecUnzip"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "SecUnzip"
#define MyAppExeName "secunzip-gui.exe"

[Setup]
AppId={{A7C3E5F1-9B2D-4E8A-6C1F-3D5B7A9E0C24}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppMutex=SecUnzipClientMutex
VersionInfoVersion={#MyAppVersion}.0
VersionInfoCompany={#MyAppPublisher}
VersionInfoDescription=SecUnzip 受控内容分发客户端
VersionInfoProductName={#MyAppName}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
; 放到输出目录（deploy\output）
OutputDir=output
OutputBaseFilename=SecUnzip-Setup
; 安装包图标（可选，无则用默认）
; SetupIconFile=assets\icon.ico
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesInstallIn64BitMode=x64compatible
; 普通用户可装（无需管理员，装到用户目录）
PrivilegesRequired=lowest
; ===== 应用列表（程序和功能）显示信息 =====
UninstallDisplayName={#MyAppName}
UninstallDisplayIcon={app}\{#MyAppExeName}
AppComments=SecUnzip 受控内容分发与内存挂载浏览工具
AppContact=
AppReadmeFile={app}\README.md
CreateUninstallRegKey=yes

[Languages]
Name: "chinesesimplified"; MessagesFile: "compiler:Default.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "附加任务："
Name: "fileassoc"; Description: "注册 .secunzip 文件关联（双击即可打开）"; GroupDescription: "附加任务："; Flags: checkedonce

[Files]
; 三个可执行文件
Source: "gui\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "target\release\secunzip.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "server\target\release\secunzip-server.exe"; DestDir: "{app}"; Flags: ignoreversion
; 部署脚本与文档
Source: "deploy\*"; DestDir: "{app}\deploy"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "ATTRIBUTION.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "assets\README.md"; DestDir: "{app}\assets"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName} 客户端"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\启动服务端"; Filename: "{app}\secunzip-server.exe"
Name: "{group}\卸载 {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{group}\归属声明与第三方许可"; Filename: "{app}\ATTRIBUTION.md"
Name: "{autodesktop}\{#MyAppName} 客户端"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
; .secunzip 文件关联（当前用户，卸载时清理）
Root: HKCU; Subkey: "Software\Classes\.secunzip"; ValueType: string; ValueData: "SecUnzip.File"; Flags: uninsdeletekey; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\SecUnzip.File"; ValueType: string; ValueData: "SecUnzip 加密文件"; Flags: uninsdeletekey; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\SecUnzip.File\DefaultIcon"; ValueType: string; ValueData: "{app}\{#MyAppExeName},0"; Tasks: fileassoc
Root: HKCU; Subkey: "Software\Classes\SecUnzip.File\shell\open\command"; ValueType: string; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: fileassoc

[Run]
; 安装完成后刷新文件关联（隐藏执行）
Filename: "{app}\{#MyAppExeName}"; Parameters: "--register"; StatusMsg: "注册文件关联..."; Flags: runhidden waituntilterminated; Tasks: fileassoc
; 可选立即启动客户端
Filename: "{app}\{#MyAppExeName}"; Description: "启动 {#MyAppName} 客户端"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; 清理运行时生成的配置/数据（可选）
Type: filesandordirs; Name: "{app}\secunzip.db"

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// SecUnzip - 受控内容分发工具
#[derive(Parser, Debug)]
#[command(name = "secunzip")]
#[command(version = "0.1.0")]
#[command(about = "打包、授权、打开受控内容", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    ///  打包文件/目录
    Pack {
        /// 源文件或目录
        #[arg(required = true)]
        sources: Vec<PathBuf>,

        /// 输出文件路径
        #[arg(short, long)]
        output: PathBuf,

        /// 服务端地址
        #[arg(long)]
        server: String,

        /// 允许临时权限申请
        #[arg(long)]
        allow_temp: bool,

        /// 生成黑盒自解压 EXE（双击运行，自查尾部标记后联网打开）
        #[arg(long)]
        blackbox: bool,
    },

    ///  打开 .secunzip 文件
    Open {
        /// 文件路径
        file: PathBuf,

        /// 用户标识（手机号/邮箱/任意ID）
        #[arg(short, long)]
        user: String,

        /// 解压目录
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// 口令（缺省依次取 SECUNZIP_PASSWORD、配置里记住的口令、标准输入一行）
        #[arg(long)]
        password: Option<String>,

        /// 取密钥成功后把口令记入本地配置，下次免输
        #[arg(long)]
        remember: bool,
    },

    ///  授权用户（管理员）
    Grant {
        /// 文件路径
        file: PathBuf,

        /// 用户标识
        #[arg(short, long)]
        user: String,

        /// 有效期（YYYYMMDD 或 Nd 表示N天）
        #[arg(short, long)]
        expires: Option<String>,

        /// 管理口令（缺省依次取 SECUNZIP_PASSWORD、配置里记住的管理口令、标准输入一行）
        #[arg(long)]
        password: Option<String>,

        /// 授权成功后把管理口令记入本地配置
        #[arg(long)]
        remember: bool,
    },

    ///  吊销用户（管理员）
    Revoke {
        /// 文件路径
        file: PathBuf,

        /// 用户标识
        #[arg(short, long)]
        user: String,
    },

    ///  申请临时权限（用户）
    Request {
        /// 文件路径
        file: PathBuf,

        /// 用户标识
        #[arg(short, long)]
        user: String,

        /// 需要多少天
        #[arg(long)]
        days: Option<i32>,

        /// 申请说明
        #[arg(long)]
        message: Option<String>,

        /// 口令（缺省依次取 SECUNZIP_PASSWORD、配置里记住的口令、标准输入一行）
        #[arg(long)]
        password: Option<String>,

        /// 申请成功后把口令记入本地配置
        #[arg(long)]
        remember: bool,
    },

    ///  查看待审批（管理员）
    Requests {
        /// 文件路径
        file: PathBuf,
    },

    ///  审批通过（管理员）
    Approve {
        /// 文件路径
        file: PathBuf,

        /// 用户标识
        #[arg(short, long)]
        user: String,

        /// 有效期（可选，覆盖申请的天数）
        #[arg(short, long)]
        expires: Option<String>,

        /// 审批口令（可选：留空 = 沿用申请者申请时设定的口令，不回退配置、不读标准输入）
        #[arg(long)]
        password: Option<String>,

        /// 本次显式填写了口令且审批成功时，把它记入配置供以后的 grant 使用
        #[arg(long)]
        remember: bool,
    },

    ///  审批拒绝（管理员）
    Deny {
        /// 文件路径
        file: PathBuf,

        /// 用户标识
        #[arg(short, long)]
        user: String,
    },
}

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

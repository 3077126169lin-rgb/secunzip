use clap::Parser;
use secunzip::cli::{args::Commands, commands, Cli};

fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Pack {
            sources,
            output,
            server,
            allow_temp,
        } => commands::cmd_pack(sources, output, server, allow_temp),
        Commands::Open { file, user, output } => commands::cmd_open(file, user, output),
        Commands::Grant {
            file,
            user,
            expires,
        } => commands::cmd_grant(file, user, expires),
        Commands::Revoke { file, user } => commands::cmd_revoke(file, user),
        Commands::Request {
            file,
            user,
            days,
            message,
        } => commands::cmd_request(file, user, days, message),
        Commands::Requests { file } => commands::cmd_requests(file),
        Commands::Approve {
            file,
            user,
            expires,
        } => commands::cmd_approve(file, user, expires),
        Commands::Deny { file, user } => commands::cmd_deny(file, user),
    };

    match result {
        Ok(()) => {}
        Err(e) => {
            eprintln!("错误: {}", e);
            std::process::exit(1);
        }
    }
}

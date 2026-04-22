use clap::{Parser, Subcommand};

mod client;
mod commands;
mod config;

#[derive(Parser)]
#[command(name = "coco", about = "CoCo Credential Gateway CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print env vars for a named token
    Env {
        /// Token name from config [tokens] section
        name: String,
        /// Also write ~/.codex/config.toml for Codex CLI
        #[arg(long)]
        codex: bool,
    },
    /// Manage gateway tokens
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
}

#[derive(Subcommand)]
enum TokenAction {
    /// Create a new named token
    Create {
        /// Human-readable name for the token
        #[arg(long)]
        name: String,
        /// Comma-separated route scopes (empty = all routes)
        #[arg(long, value_delimiter = ',')]
        scope: Vec<String>,
    },
    /// List all tokens
    Ls,
    /// Revoke a token by name
    Revoke {
        /// Token name to revoke
        name: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Env { name, codex } => commands::env::run(&name, codex)?,
        Commands::Token { action } => match action {
            TokenAction::Create { name, scope } => commands::token::create(&name, &scope).await?,
            TokenAction::Ls => commands::token::list().await?,
            TokenAction::Revoke { name } => commands::token::revoke(&name).await?,
        },
    }
    Ok(())
}
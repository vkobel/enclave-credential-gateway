use clap::{Parser, Subcommand};

mod client;
mod commands;
mod config;
#[cfg(test)]
mod test_support;
mod tooling;

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
    /// Configure specific tools
    Tool {
        #[command(subcommand)]
        action: ToolAction,
    },
    /// Manage gateway tokens
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
}

#[derive(Subcommand)]
enum ToolAction {
    /// Print env vars for a specific tool
    Env {
        /// Tool adapter name
        tool: String,
        /// Token name from config [tokens] section
        name: String,
    },
    /// Render a tool config artifact to stdout
    Render {
        /// Tool adapter name
        tool: String,
        /// Token name from config [tokens] section
        name: String,
    },
    /// Install a tool config artifact to its default location
    Install {
        /// Tool adapter name
        tool: String,
        /// Token name from config [tokens] section
        name: String,
    },
}

#[derive(Subcommand)]
enum TokenAction {
    /// Create a new named token
    Create {
        /// Human-readable name for the token
        #[arg(long)]
        name: String,
        /// Comma-separated route scopes (empty = all current and future routes)
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
        Commands::Tool { action } => match action {
            ToolAction::Env { tool, name } => commands::tool::run_env(&tool, &name)?,
            ToolAction::Render { tool, name } => commands::tool::run_render(&tool, &name)?,
            ToolAction::Install { tool, name } => commands::tool::run_install(&tool, &name)?,
        },
        Commands::Token { action } => match action {
            TokenAction::Create { name, scope } => commands::token::create(&name, &scope).await?,
            TokenAction::Ls => commands::token::list().await?,
            TokenAction::Revoke { name } => commands::token::revoke(&name).await?,
        },
    }
    Ok(())
}

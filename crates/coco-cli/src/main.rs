use clap::{Parser, Subcommand};

mod client;
mod commands;
mod config;
mod secure_file;
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
    /// Configure shell and tools for a token
    Activate {
        /// Token name from config [tokens] section
        name: String,
        /// Print eval-safe shell exports instead of launching an activated subshell
        #[arg(long)]
        eval: bool,
        /// Describe activation actions without applying them
        #[arg(long)]
        describe: bool,
        /// Restrict to one or more tool adapters
        #[arg(long, value_delimiter = ',')]
        tool: Vec<String>,
        /// Restrict emitted route-specific entries to one route
        #[arg(long)]
        route: Option<String>,
    },
    /// Manage gateway tokens
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
    /// Internal Git credential helper
    #[command(name = "git-credential", hide = true)]
    GitCredential {
        /// Token name from config [tokens] section
        name: String,
        /// Git credential operation: get, store, or erase
        operation: String,
    },
}

#[derive(Subcommand)]
enum TokenAction {
    /// Create a new named token
    Create {
        /// Human-readable name for the token
        #[arg(long)]
        name: String,
        /// Comma-separated route scopes
        #[arg(long, value_delimiter = ',')]
        scope: Vec<String>,
        /// Create a token that can access all current and future routes
        #[arg(long)]
        all_routes: bool,
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
        Commands::Activate {
            name,
            eval,
            describe,
            tool,
            route,
        } => commands::activate::run(&name, eval, describe, &tool, route.as_deref())?,
        Commands::Token { action } => match action {
            TokenAction::Create {
                name,
                scope,
                all_routes,
            } => commands::token::create(&name, &scope, all_routes).await?,
            TokenAction::Ls => commands::token::list().await?,
            TokenAction::Revoke { name } => commands::token::revoke(&name).await?,
        },
        Commands::GitCredential { name, operation } => {
            commands::git_credential::run(&name, &operation)?
        }
    }
    Ok(())
}

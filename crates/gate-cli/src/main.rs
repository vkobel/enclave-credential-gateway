use clap::{Parser, Subcommand};

mod client;
mod commands;
mod config;
mod secure_file;
#[cfg(test)]
mod test_support;
mod tooling;
mod transport;

#[derive(Parser)]
#[command(name = "gate", about = "Enclave Credential Gateway CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage gateway state through the admin API
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
    /// Configure shell and tools for a token
    Activate {
        /// Token name from config [tokens] section
        name: String,
        /// Print eval-safe shell exports instead of launching an activated subshell
        #[arg(long)]
        eval: bool,
        /// Restrict to one or more tool adapters
        #[arg(long, value_delimiter = ',')]
        tool: Vec<String>,
        /// Restrict emitted route-specific entries to one route
        #[arg(long)]
        route: Option<String>,
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
enum AdminAction {
    /// Manage gateway phantom tokens
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
    /// Manage registered service credentials
    Creds {
        #[command(subcommand)]
        action: CredsAction,
    },
}

#[derive(Subcommand)]
enum TokenAction {
    /// Create a new named gateway token
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
        /// Bind a route to a registered service credential (format: route=cred-name).
        /// The value is sent over the encrypted admin channel.
        #[arg(long = "cred", value_name = "ROUTE=CRED", value_parser = parse_cred_binding)]
        creds: Vec<(String, String)>,
    },
    /// List gateway tokens
    Ls,
    /// Revoke a gateway token by name
    Revoke {
        /// Token name to revoke
        name: String,
    },
}

#[derive(Subcommand)]
enum CredsAction {
    /// Register a service credential (value sent over the encrypted admin channel)
    Register {
        /// Service this credential belongs to (e.g. openai, github)
        service: String,
        /// The secret credential value
        value: String,
        /// Name for this credential; defaults to <service>
        #[arg(long)]
        name: Option<String>,
    },
    /// List registered service credentials
    Ls,
    /// Remove a registered service credential by name
    Rm {
        /// Credential name to remove
        name: String,
    },
}

/// Parse a `route=cred-name` binding for `--cred`.
fn parse_cred_binding(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("'{}': expected format route=cred-name", s))?;
    let route = &s[..pos];
    let cred = &s[pos + 1..];
    if route.is_empty() {
        return Err(format!("'{}': route part must not be empty", s));
    }
    if cred.is_empty() {
        return Err(format!("'{}': cred-name part must not be empty", s));
    }
    Ok((route.to_string(), cred.to_string()))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Activate {
            name,
            eval,
            tool,
            route,
        } => commands::activate::run(&name, eval, &tool, route.as_deref())?,
        Commands::Admin { action } => match action {
            AdminAction::Token { action } => match action {
                TokenAction::Create {
                    name,
                    scope,
                    all_routes,
                    creds,
                } => commands::token::create(&name, &scope, all_routes, &creds).await?,
                TokenAction::Ls => commands::token::list().await?,
                TokenAction::Revoke { name } => commands::token::revoke(&name).await?,
            },
            AdminAction::Creds { action } => match action {
                CredsAction::Register {
                    service,
                    value,
                    name,
                } => {
                    let name = name.unwrap_or_else(|| service.clone());
                    commands::creds::register(&name, &service, &value).await?
                }
                CredsAction::Ls => commands::creds::list().await?,
                CredsAction::Rm { name } => commands::creds::rm(&name).await?,
            },
        },
        Commands::GitCredential { name, operation } => {
            commands::git_credential::run(&name, &operation)?
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_cred_binding;

    #[test]
    fn parse_cred_binding_valid() {
        let (route, cred) = parse_cred_binding("openai=sk-prod").unwrap();
        assert_eq!(route, "openai");
        assert_eq!(cred, "sk-prod");
    }

    #[test]
    fn parse_cred_binding_missing_equals() {
        let err = parse_cred_binding("openai-sk-prod").unwrap_err();
        assert!(err.contains("route=cred-name"), "got: {err}");
    }

    #[test]
    fn parse_cred_binding_empty_route() {
        let err = parse_cred_binding("=sk-prod").unwrap_err();
        assert!(err.contains("route part must not be empty"), "got: {err}");
    }

    #[test]
    fn parse_cred_binding_empty_cred() {
        let err = parse_cred_binding("openai=").unwrap_err();
        assert!(
            err.contains("cred-name part must not be empty"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_cred_binding_value_with_equals() {
        // Only splits on the first '=', so values with '=' in the cred name are preserved
        let (route, cred) = parse_cred_binding("github=my=cred").unwrap();
        assert_eq!(route, "github");
        assert_eq!(cred, "my=cred");
    }
}

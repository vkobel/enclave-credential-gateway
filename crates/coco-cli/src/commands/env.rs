use crate::config::Config;
use crate::tooling;
use anyhow::{anyhow, Result};

pub fn run(name: &str, codex: bool) -> Result<()> {
    let config = Config::load()?;
    for line in tooling::render_tool_env(&config, "shell", name)? {
        println!("{line}");
    }

    if codex && should_install_codex(&config, name)? {
        let _ = tooling::install_tool_file(&config, "codex", name)?;
    }

    Ok(())
}

fn should_install_codex(config: &Config, name: &str) -> Result<bool> {
    let entry = config
        .tokens
        .get(name)
        .ok_or_else(|| anyhow!("Token '{}' not found in config", name))?;
    Ok(entry.allows_route("openai"))
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::config::{Config, TokenEntry};
    use crate::test_support::with_temp_home;
    use std::collections::HashMap;

    #[test]
    fn codex_flag_is_noop_without_openai_scope() {
        with_temp_home(|temp| {
            let mut tokens = HashMap::new();
            tokens.insert(
                "laptop".to_string(),
                TokenEntry {
                    token: "ccgw_test".to_string(),
                    scope: vec!["github".to_string()],
                },
            );

            Config {
                gateway_url: "https://gw.example.com".to_string(),
                admin_token: None,
                tokens,
            }
            .save()
            .unwrap();

            run("laptop", true).unwrap();

            assert!(!temp.path().join(".codex/config.toml").exists());
        });
    }
}

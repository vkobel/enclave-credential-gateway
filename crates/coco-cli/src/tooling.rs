use crate::config::{Config, TokenEntry};
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

const MANIFEST_YAML: &str = include_str!("../../../profiles/coco.yaml");

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Manifest {
    #[serde(default)]
    routes: BTreeMap<String, serde_yaml::Value>,
    #[serde(default)]
    tools: BTreeMap<String, ToolAdapter>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ToolAdapter {
    #[serde(default)]
    pub routes: Vec<String>,
    #[serde(default)]
    pub git_credential_helper: bool,
    #[serde(default)]
    pub env: Vec<ToolEnvVar>,
    #[serde(default)]
    pub files: Vec<ToolFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolEnvVar {
    #[serde(default)]
    pub requires_route: Option<String>,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolFile {
    pub id: String,
    #[serde(default)]
    pub requires_route: Option<String>,
    #[serde(default)]
    pub managed_path: Option<String>,
    #[serde(default)]
    pub generated_path: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationMode {
    Generated,
    Describe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvExport {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Activation {
    pub exports: Vec<EnvExport>,
    pub files: Vec<PathBuf>,
}

impl Activation {
    pub fn shell_lines(&self) -> Vec<String> {
        self.exports
            .iter()
            .map(|export| format!("export {}={}", export.key, shell_word(&export.value)))
            .chain(
                self.files
                    .iter()
                    .map(|path| format!("# wrote {}", path.display())),
            )
            .collect()
    }

    pub fn describe_lines(&self) -> Vec<String> {
        self.exports
            .iter()
            .map(|export| format!("export {}={}", export.key, export.value))
            .chain(
                self.files
                    .iter()
                    .map(|path| format!("write {}", path.display())),
            )
            .collect()
    }
}

struct ToolContext<'a> {
    token_name: &'a str,
    token_value: &'a str,
    gateway_url: &'a str,
    gateway_host: String,
    entry: &'a TokenEntry,
    generated_root: PathBuf,
    route_filter: Option<&'a str>,
}

pub fn activate(
    config: &Config,
    token_name: &str,
    tool_filter: Option<&[String]>,
    route_filter: Option<&str>,
    mode: ActivationMode,
) -> Result<Activation> {
    let manifest = load_manifest()?;
    if let Some(route_filter) = route_filter {
        if !manifest.routes.contains_key(route_filter) {
            bail!(
                "Unknown route '{}'. Known routes: {}",
                route_filter,
                manifest
                    .routes
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    if let Some(tool_filter) = tool_filter {
        for requested in tool_filter {
            if !manifest.tools.contains_key(requested) {
                bail!("Unknown tool adapter '{}'", requested);
            }
        }
    }
    let mut activation = Activation::default();

    for (tool, adapter) in manifest.tools {
        if let Some(tool_filter) = tool_filter {
            if !tool_filter.iter().any(|requested| requested == &tool) {
                continue;
            }
        }

        let ctx = ToolContext::new(config, token_name, &tool, route_filter)?;
        if !tool_applies(&adapter, &ctx) {
            if tool_filter.is_some() {
                bail!(
                    "Token '{}' is not scoped for any route required by tool '{}'",
                    token_name,
                    tool
                );
            }
            continue;
        }

        let managed_files = materialize_managed_files(&tool, &adapter, &ctx, mode)?;
        activation.files.extend(managed_files.values().cloned());
        for env in &adapter.env {
            if !ctx.has_route(env.requires_route.as_deref()) {
                continue;
            }
            let value = render_template(&env.value, &ctx, &managed_files)?;
            push_export(&mut activation.exports, env.key.clone(), value);
        }

        if adapter.git_credential_helper && ctx.has_route(Some("github")) {
            let gitconfig = materialize_git_credential_config(&tool, &ctx, mode)?;
            activation.files.push(gitconfig.clone());
            push_export(
                &mut activation.exports,
                "GIT_CONFIG_GLOBAL".to_string(),
                gitconfig.display().to_string(),
            );
        }

        activation.files.extend(install_tool_files(
            &tool,
            &adapter,
            &ctx,
            &managed_files,
            mode,
        )?);
    }

    Ok(activation)
}

fn push_export(exports: &mut Vec<EnvExport>, key: String, value: String) {
    if let Some(pos) = exports.iter().position(|export| export.key == key) {
        exports.remove(pos);
    }
    exports.push(EnvExport { key, value });
}

fn tool_applies(adapter: &ToolAdapter, ctx: &ToolContext<'_>) -> bool {
    if adapter.routes.is_empty() {
        true
    } else {
        adapter
            .routes
            .iter()
            .any(|route| ctx.has_route(Some(route.as_str())))
    }
}

fn install_tool_files(
    tool: &str,
    adapter: &ToolAdapter,
    ctx: &ToolContext<'_>,
    managed_files: &HashMap<String, PathBuf>,
    mode: ActivationMode,
) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for file in &adapter.files {
        if !ctx.has_route(file.requires_route.as_deref()) {
            continue;
        }
        let target = file.generated_path.as_ref();
        let Some(target) = target else {
            continue;
        };
        paths.push(write_tool_file(
            tool,
            file,
            target,
            ctx,
            managed_files,
            mode,
        )?);
    }
    Ok(paths)
}

pub fn known_routes() -> Result<Vec<String>> {
    Ok(load_manifest()?.routes.into_keys().collect())
}

pub fn load_manifest() -> Result<Manifest> {
    Manifest::from_str(MANIFEST_YAML)
}

impl Manifest {
    pub fn from_str(contents: &str) -> Result<Self> {
        serde_yaml::from_str(contents).context("Failed to parse embedded CoCo manifest")
    }
}

fn materialize_managed_files(
    tool: &str,
    adapter: &ToolAdapter,
    ctx: &ToolContext<'_>,
    mode: ActivationMode,
) -> Result<HashMap<String, PathBuf>> {
    let mut managed_files = HashMap::new();

    for file in &adapter.files {
        if !ctx.has_route(file.requires_route.as_deref()) {
            continue;
        }
        let Some(relative_path) = &file.managed_path else {
            continue;
        };

        let path = ctx
            .generated_root
            .join(validate_managed_path(relative_path)?);
        let content = render_template(&file.content, ctx, &managed_files)?;
        if mode != ActivationMode::Describe {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)
                .with_context(|| format!("Failed to write managed file for tool '{}'", tool))?;
        }
        managed_files.insert(file.id.clone(), path);
    }

    Ok(managed_files)
}

fn write_tool_file(
    tool: &str,
    file: &ToolFile,
    target: &str,
    ctx: &ToolContext<'_>,
    managed_files: &HashMap<String, PathBuf>,
    mode: ActivationMode,
) -> Result<PathBuf> {
    let target = render_template(target, ctx, managed_files)?;
    let path = Config::expand_home(&target);
    let content = render_template(&file.content, ctx, managed_files)?;

    if mode != ActivationMode::Describe {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write config file for tool '{}'", tool))?;
    }

    Ok(path)
}

fn render_template(
    template: &str,
    ctx: &ToolContext<'_>,
    managed_files: &HashMap<String, PathBuf>,
) -> Result<String> {
    let mut rendered = String::new();
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        rendered.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail
            .find("}}")
            .ok_or_else(|| anyhow!("Unclosed template marker in '{}'", template))?;
        let token = tail[..end].trim();
        let value = match token {
            "token" => ctx.token_value.to_string(),
            "token_name" => ctx.token_name.to_string(),
            "gateway_url" => ctx.gateway_url.to_string(),
            "gateway_host" => ctx.gateway_host.clone(),
            "generated_root" => ctx.generated_root.display().to_string(),
            _ => {
                if let Some(route) = token.strip_prefix("route_url:") {
                    format!("{}/{}", ctx.gateway_url, route)
                } else if let Some(file_id) = token.strip_prefix("managed_file:") {
                    managed_files
                        .get(file_id)
                        .map(|path| path.display().to_string())
                        .ok_or_else(|| anyhow!("Unknown managed file reference '{}'", file_id))?
                } else {
                    bail!("Unknown template variable '{{{{{}}}}}'", token);
                }
            }
        };
        rendered.push_str(&value);
        rest = &tail[end + 2..];
    }

    rendered.push_str(rest);
    Ok(rendered)
}

fn materialize_git_credential_config(
    tool: &str,
    ctx: &ToolContext<'_>,
    mode: ActivationMode,
) -> Result<PathBuf> {
    let path = ctx.generated_root.join("gitconfig");
    let helper = format!("!coco git-credential {}", shell_word(ctx.token_name));
    let content = format!(
        "[include]\n    path = ~/.gitconfig\n\n[credential \"{}\"]\n    helper =\n    helper = \"{}\"\n",
        ctx.gateway_url,
        helper.replace('"', "\\\"")
    );

    if mode != ActivationMode::Describe {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content).with_context(|| {
            format!("Failed to write Git credential config for tool '{}'", tool)
        })?;
    }
    Ok(path)
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn shell_word(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'@' | b'%' | b'+' | b'=' | b':' | b',' | b'.' | b'/' | b'-'))
    {
        value.to_string()
    } else {
        shell_quote(value)
    }
}



fn validate_managed_path(path: &str) -> Result<PathBuf> {
    let managed_path = Path::new(path);
    if managed_path.is_absolute()
        || managed_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!(
            "Invalid managed_path '{}': path must be relative and stay under the generated directory",
            path
        );
    }
    Ok(managed_path.to_path_buf())
}

impl<'a> ToolContext<'a> {
    fn new(
        config: &'a Config,
        token_name: &'a str,
        tool: &'a str,
        route_filter: Option<&'a str>,
    ) -> Result<Self> {
        let entry = config
            .tokens
            .get(token_name)
            .ok_or_else(|| anyhow!("Token '{}' not found in config", token_name))?;
        let gateway_url = config.gateway_url.trim_end_matches('/');
        Ok(Self {
            token_name,
            token_value: &entry.token,
            gateway_url,
            gateway_host: host_only(gateway_url),
            entry,
            generated_root: Config::generated_dir().join(tool).join(token_name),
            route_filter,
        })
    }

    fn has_route(&self, route: Option<&str>) -> bool {
        match route {
            None => true,
            Some(route) => {
                if self
                    .route_filter
                    .is_some_and(|route_filter| route_filter != route)
                {
                    return false;
                }
                self.entry.allows_route(route)
            }
        }
    }
}

fn host_only(url: &str) -> String {
    let stripped = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    stripped.split('/').next().unwrap_or(stripped).to_string()
}

#[cfg(test)]
mod tests {
    use super::{activate, validate_managed_path, ActivationMode, EnvExport};
    use crate::config::{Config, TokenEntry};
    use crate::test_support::with_temp_config_root;
    use std::collections::HashMap;

    fn config_with_scope(scope: &[&str]) -> Config {
        config_with_token(scope, false)
    }

    fn config_with_all_routes() -> Config {
        config_with_token(&[], true)
    }

    fn config_with_token(scope: &[&str], all_routes: bool) -> Config {
        let mut tokens = HashMap::new();
        tokens.insert(
            "laptop".to_string(),
            TokenEntry {
                token: "ccgw_test".to_string(),
                scope: scope.iter().map(|scope| scope.to_string()).collect(),
                all_routes,
            },
        );

        Config {
            gateway_url: "https://gw.example.com".to_string(),
            admin_token: None,
            tokens,
        }
    }

    #[test]
    fn shell_adapter_preserves_existing_exports() {
        with_temp_config_root(|_temp| {
            let config = config_with_scope(&["anthropic", "openai", "github", "ollama", "httpbin"]);
            let activation = activate(
                &config,
                "laptop",
                Some(&["shell".to_string()]),
                None,
                ActivationMode::Generated,
            )
            .unwrap();

            assert_eq!(
                activation.exports,
                vec![
                    EnvExport {
                        key: "ANTHROPIC_BASE_URL".to_string(),
                        value: "https://gw.example.com/anthropic".to_string()
                    },
                    EnvExport {
                        key: "ANTHROPIC_API_KEY".to_string(),
                        value: "ccgw_test".to_string()
                    },
                    EnvExport {
                        key: "OPENAI_BASE_URL".to_string(),
                        value: "https://gw.example.com/openai".to_string()
                    },
                    EnvExport {
                        key: "OPENAI_API_KEY".to_string(),
                        value: "ccgw_test".to_string()
                    },
                    EnvExport {
                        key: "GH_HOST".to_string(),
                        value: "gw.example.com".to_string()
                    },
                    EnvExport {
                        key: "GH_ENTERPRISE_TOKEN".to_string(),
                        value: "ccgw_test".to_string()
                    },
                    EnvExport {
                        key: "GH_TOKEN".to_string(),
                        value: "ccgw_test".to_string()
                    },
                    EnvExport {
                        key: "OLLAMA_HOST".to_string(),
                        value: "https://gw.example.com/ollama".to_string()
                    },
                    EnvExport {
                        key: "HTTPBIN_TOKEN".to_string(),
                        value: "ccgw_test".to_string()
                    },
                ]
            );
        });
    }

    #[test]
    fn gh_adapter_exports_enterprise_credentials() {
        with_temp_config_root(|temp| {
            let config = config_with_scope(&["github"]);
            let activation = activate(
                &config,
                "laptop",
                Some(&["gh".to_string()]),
                None,
                ActivationMode::Generated,
            )
            .unwrap();
            let gitconfig = temp
                .path()
                .join(".config/coco/generated/gh/laptop/gitconfig");

            assert_eq!(
                activation.exports,
                vec![
                    EnvExport {
                        key: "GH_HOST".to_string(),
                        value: "gw.example.com".to_string()
                    },
                    EnvExport {
                        key: "GH_ENTERPRISE_TOKEN".to_string(),
                        value: "ccgw_test".to_string()
                    },
                    EnvExport {
                        key: "GH_TOKEN".to_string(),
                        value: "ccgw_test".to_string()
                    },
                    EnvExport {
                        key: "GIT_CONFIG_GLOBAL".to_string(),
                        value: gitconfig.display().to_string()
                    },
                ]
            );
            let contents = std::fs::read_to_string(gitconfig).unwrap();
            assert!(contents.contains("[include]\n    path = ~/.gitconfig"));
            assert!(contents.contains("[credential \"https://gw.example.com\"]"));
            assert!(contents.contains("    helper =\n"));
            assert!(contents.contains("    helper = \"!coco git-credential laptop\""));
        });
    }

    #[test]
    fn all_routes_token_activates_scoped_tools() {
        with_temp_config_root(|_temp| {
            let config = config_with_all_routes();
            let activation = activate(
                &config,
                "laptop",
                Some(&["gh".to_string()]),
                None,
                ActivationMode::Generated,
            )
            .unwrap();

            assert!(activation
                .exports
                .iter()
                .any(|export| export.key == "GH_HOST" && export.value == "gw.example.com"));
            assert!(activation
                .exports
                .iter()
                .any(|export| export.key == "GIT_CONFIG_GLOBAL"));
        });
    }

    #[test]
    fn opencode_env_materializes_managed_config() {
        with_temp_config_root(|temp| {
            let config = config_with_scope(&["openai", "anthropic"]);
            let activation = activate(
                &config,
                "laptop",
                Some(&["opencode".to_string()]),
                None,
                ActivationMode::Generated,
            )
            .unwrap();

            let expected_path = temp
                .path()
                .join(".config/coco/generated/opencode/laptop/opencode.json");
            assert!(expected_path.exists());
            assert!(activation.exports.contains(&EnvExport {
                key: "OPENCODE_CONFIG".to_string(),
                value: expected_path.display().to_string()
            }));
            assert!(activation.exports.contains(&EnvExport {
                key: "OPENAI_API_KEY".to_string(),
                value: "ccgw_test".to_string()
            }));
            assert!(activation.exports.contains(&EnvExport {
                key: "ANTHROPIC_API_KEY".to_string(),
                value: "ccgw_test".to_string()
            }));
        });
    }

    #[test]
    fn codex_generated_activation_sets_codex_home() {
        with_temp_config_root(|temp| {
            let config = config_with_scope(&["openai"]);
            let activation = activate(
                &config,
                "laptop",
                Some(&["codex".to_string()]),
                None,
                ActivationMode::Generated,
            )
            .unwrap();

            let codex_home = temp.path().join(".config/coco/generated/codex/laptop/home");
            assert!(activation.exports.contains(&EnvExport {
                key: "CODEX_HOME".to_string(),
                value: codex_home.display().to_string()
            }));

            let expected = codex_home.join("config.toml");
            let contents = std::fs::read_to_string(expected).unwrap();
            assert!(contents.contains("model_provider = \"openai\""));
            assert!(contents.contains("openai_base_url = \"https://gw.example.com/openai/v1\""));

            let auth_path = codex_home.join("auth.json");
            let auth = std::fs::read_to_string(auth_path).unwrap();
            assert!(auth.contains("\"auth_mode\": \"apikey\""));
            assert!(auth.contains("\"OPENAI_API_KEY\": \"ccgw_test\""));
        });
    }

    #[test]
    fn managed_path_rejects_parent_traversal() {
        let error = validate_managed_path("../escape.txt").unwrap_err();
        assert!(error
            .to_string()
            .contains("Invalid managed_path '../escape.txt'"));
    }

    #[test]
    fn managed_path_rejects_absolute_paths() {
        let error = validate_managed_path("/tmp/escape.txt").unwrap_err();
        assert!(error
            .to_string()
            .contains("Invalid managed_path '/tmp/escape.txt'"));
    }
}

use crate::config::{Config, TokenEntry};
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

const BUILTIN_ADAPTERS_TOML: &str = include_str!("../builtin-tools.toml");

#[derive(Debug, Clone, Deserialize, Default)]
struct ToolRegistryFile {
    #[serde(default)]
    adapters: HashMap<String, ToolAdapter>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ToolAdapter {
    #[serde(default)]
    pub experimental: bool,
    #[serde(default)]
    pub git_credential_helper: bool,
    #[serde(default)]
    pub default_render_file: Option<String>,
    #[serde(default)]
    pub default_install_file: Option<String>,
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
    pub install_path: Option<String>,
    pub content: String,
}

struct ToolContext<'a> {
    tool: &'a str,
    token_name: &'a str,
    token_value: &'a str,
    gateway_url: &'a str,
    gateway_host: String,
    entry: &'a TokenEntry,
    generated_root: PathBuf,
}

pub fn get_tool_adapter(name: &str) -> Result<ToolAdapter> {
    load_registry()?
        .remove(name)
        .ok_or_else(|| anyhow!("Unknown tool adapter '{}'", name))
}

pub fn render_tool_env(config: &Config, tool: &str, token_name: &str) -> Result<Vec<String>> {
    let adapter = get_tool_adapter(tool)?;
    let ctx = ToolContext::new(config, token_name, tool)?;
    let managed_files = materialize_managed_files(tool, &adapter, &ctx)?;

    let mut exports = Vec::new();
    for env in &adapter.env {
        if !ctx.has_route(env.requires_route.as_deref()) {
            continue;
        }
        let value = render_template(&env.value, &ctx, &managed_files)?;
        exports.push(format!("export {}={}", env.key, value));
    }

    if adapter.git_credential_helper && ctx.has_route(Some("github")) {
        exports.extend(render_git_credential_helper_env(&ctx));
    }

    Ok(exports)
}

pub fn render_tool_file(config: &Config, tool: &str, token_name: &str) -> Result<String> {
    let adapter = get_tool_adapter(tool)?;
    let ctx = ToolContext::new(config, token_name, tool)?;
    let managed_files = materialize_managed_files(tool, &adapter, &ctx)?;
    let file = default_file(&adapter, FilePurpose::Render)?;
    ensure_route_allowed(&ctx, file)?;
    render_template(&file.content, &ctx, &managed_files)
}

pub fn install_tool_file(config: &Config, tool: &str, token_name: &str) -> Result<PathBuf> {
    let adapter = get_tool_adapter(tool)?;
    let ctx = ToolContext::new(config, token_name, tool)?;
    let managed_files = materialize_managed_files(tool, &adapter, &ctx)?;
    let file = default_file(&adapter, FilePurpose::Install)?;
    ensure_route_allowed(&ctx, file)?;

    let install_path = file
        .install_path
        .as_ref()
        .ok_or_else(|| anyhow!("Tool '{}' does not define an install path", tool))?;
    let install_path = render_template(install_path, &ctx, &managed_files)?;
    let install_path = Config::expand_home(&install_path);
    let content = render_template(&file.content, &ctx, &managed_files)?;

    if let Some(parent) = install_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&install_path, content)?;

    Ok(install_path)
}

fn load_registry() -> Result<HashMap<String, ToolAdapter>> {
    let builtin: ToolRegistryFile =
        toml::from_str(BUILTIN_ADAPTERS_TOML).context("Failed to parse built-in tool adapters")?;
    let mut adapters = builtin.adapters;

    let user_path = Config::tools_path();
    if user_path.exists() {
        let user_contents = std::fs::read_to_string(&user_path)
            .with_context(|| format!("Failed to read {}", user_path.display()))?;
        let user_registry: ToolRegistryFile = toml::from_str(&user_contents)
            .with_context(|| format!("Failed to parse {}", user_path.display()))?;
        adapters.extend(user_registry.adapters);
    }

    Ok(adapters)
}

fn materialize_managed_files(
    tool: &str,
    adapter: &ToolAdapter,
    ctx: &ToolContext<'_>,
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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write managed file for tool '{}'", tool))?;
        managed_files.insert(file.id.clone(), path);
    }

    Ok(managed_files)
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

fn render_git_credential_helper_env(ctx: &ToolContext<'_>) -> Vec<String> {
    let key = format!("credential.{}.helper", ctx.gateway_url);
    let helper = format!("!coco git-credential {}", shell_word(ctx.token_name));

    vec![
        r#"__coco_git_config_original_count="${GIT_CONFIG_COUNT:-0}""#.to_string(),
        r#"__coco_git_config_next=0"#.to_string(),
        r#"__coco_git_config_i=0"#.to_string(),
        format!("__coco_git_config_key={}", shell_quote(&key)),
        format!("__coco_git_config_value={}", shell_quote(&helper)),
        r#"while [ "$__coco_git_config_i" -lt "$__coco_git_config_original_count" ]; do"#
            .to_string(),
        r#"  eval "__coco_git_config_existing_key=\${GIT_CONFIG_KEY_${__coco_git_config_i}-}""#
            .to_string(),
        r#"  eval "__coco_git_config_existing_value=\${GIT_CONFIG_VALUE_${__coco_git_config_i}-}""#
            .to_string(),
        r#"  if [ "$__coco_git_config_existing_key" != "$__coco_git_config_key" ]; then"#
            .to_string(),
        r#"    eval "export GIT_CONFIG_KEY_${__coco_git_config_next}=\$__coco_git_config_existing_key""#
            .to_string(),
        r#"    eval "export GIT_CONFIG_VALUE_${__coco_git_config_next}=\$__coco_git_config_existing_value""#
            .to_string(),
        r#"    __coco_git_config_next=$((__coco_git_config_next + 1))"#.to_string(),
        r#"  fi"#.to_string(),
        r#"  __coco_git_config_i=$((__coco_git_config_i + 1))"#.to_string(),
        r#"done"#.to_string(),
        r#"eval "export GIT_CONFIG_KEY_${__coco_git_config_next}=\$__coco_git_config_key""#
            .to_string(),
        r#"eval "export GIT_CONFIG_VALUE_${__coco_git_config_next}=""#.to_string(),
        r#"__coco_git_config_next=$((__coco_git_config_next + 1))"#.to_string(),
        r#"eval "export GIT_CONFIG_KEY_${__coco_git_config_next}=\$__coco_git_config_key""#
            .to_string(),
        r#"eval "export GIT_CONFIG_VALUE_${__coco_git_config_next}=\$__coco_git_config_value""#
            .to_string(),
        r#"export GIT_CONFIG_COUNT=$((__coco_git_config_next + 1))"#.to_string(),
        r#"unset __coco_git_config_original_count __coco_git_config_next __coco_git_config_i __coco_git_config_key __coco_git_config_value __coco_git_config_existing_key __coco_git_config_existing_value"#
            .to_string(),
    ]
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

fn default_file(adapter: &ToolAdapter, purpose: FilePurpose) -> Result<&ToolFile> {
    let file_id = match purpose {
        FilePurpose::Render => adapter
            .default_render_file
            .as_deref()
            .ok_or_else(|| anyhow!("Tool does not define a default renderable file"))?,
        FilePurpose::Install => adapter
            .default_install_file
            .as_deref()
            .ok_or_else(|| anyhow!("Tool does not define a default installable file"))?,
    };

    adapter
        .files
        .iter()
        .find(|file| file.id == file_id)
        .ok_or_else(|| anyhow!("Tool references unknown file '{}'", file_id))
}

fn ensure_route_allowed(ctx: &ToolContext<'_>, file: &ToolFile) -> Result<()> {
    if ctx.has_route(file.requires_route.as_deref()) {
        Ok(())
    } else {
        let route = file.requires_route.as_deref().unwrap_or("<unspecified>");
        bail!(
            "Token '{}' is not scoped for route '{}' required by tool '{}'",
            ctx.token_name,
            route,
            ctx.tool
        )
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
    fn new(config: &'a Config, token_name: &'a str, tool: &'a str) -> Result<Self> {
        let entry = config
            .tokens
            .get(token_name)
            .ok_or_else(|| anyhow!("Token '{}' not found in config", token_name))?;
        let gateway_url = config.gateway_url.trim_end_matches('/');
        Ok(Self {
            tool,
            token_name,
            token_value: &entry.token,
            gateway_url,
            gateway_host: host_only(gateway_url),
            entry,
            generated_root: Config::generated_dir().join(tool).join(token_name),
        })
    }

    fn has_route(&self, route: Option<&str>) -> bool {
        match route {
            None => true,
            Some(route) => self.entry.allows_route(route),
        }
    }
}

fn host_only(url: &str) -> String {
    let stripped = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    stripped.split('/').next().unwrap_or(stripped).to_string()
}

enum FilePurpose {
    Render,
    Install,
}

#[cfg(test)]
mod tests {
    use super::{get_tool_adapter, install_tool_file, render_tool_env, render_tool_file};
    use crate::config::{Config, TokenEntry};
    use crate::test_support::with_temp_config_root;
    use std::collections::HashMap;

    fn config_with_scope(scope: &[&str]) -> Config {
        let mut tokens = HashMap::new();
        tokens.insert(
            "laptop".to_string(),
            TokenEntry {
                token: "ccgw_test".to_string(),
                scope: scope.iter().map(|scope| scope.to_string()).collect(),
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
            let exports = render_tool_env(&config, "shell", "laptop").unwrap();

            assert_eq!(
                exports,
                vec![
                    "export ANTHROPIC_BASE_URL=https://gw.example.com/anthropic",
                    "export ANTHROPIC_API_KEY=ccgw_test",
                    "export OPENAI_BASE_URL=https://gw.example.com/openai",
                    "export OPENAI_API_KEY=ccgw_test",
                    "export GH_HOST=gw.example.com",
                    "export GH_ENTERPRISE_TOKEN=ccgw_test",
                    "export GH_TOKEN=ccgw_test",
                    "export OLLAMA_HOST=https://gw.example.com/ollama",
                    "export HTTPBIN_TOKEN=ccgw_test",
                ]
            );
        });
    }

    #[test]
    fn gh_adapter_exports_enterprise_credentials() {
        with_temp_config_root(|_temp| {
            let config = config_with_scope(&["github"]);
            let exports = render_tool_env(&config, "gh", "laptop").unwrap();

            assert_eq!(
                exports,
                vec![
                    "export GH_HOST=gw.example.com",
                    "export GH_ENTERPRISE_TOKEN=ccgw_test",
                    "export GH_TOKEN=ccgw_test",
                    "__coco_git_config_original_count=\"${GIT_CONFIG_COUNT:-0}\"",
                    "__coco_git_config_next=0",
                    "__coco_git_config_i=0",
                    "__coco_git_config_key='credential.https://gw.example.com.helper'",
                    "__coco_git_config_value='!coco git-credential laptop'",
                    "while [ \"$__coco_git_config_i\" -lt \"$__coco_git_config_original_count\" ]; do",
                    "  eval \"__coco_git_config_existing_key=\\${GIT_CONFIG_KEY_${__coco_git_config_i}-}\"",
                    "  eval \"__coco_git_config_existing_value=\\${GIT_CONFIG_VALUE_${__coco_git_config_i}-}\"",
                    "  if [ \"$__coco_git_config_existing_key\" != \"$__coco_git_config_key\" ]; then",
                    "    eval \"export GIT_CONFIG_KEY_${__coco_git_config_next}=\\$__coco_git_config_existing_key\"",
                    "    eval \"export GIT_CONFIG_VALUE_${__coco_git_config_next}=\\$__coco_git_config_existing_value\"",
                    "    __coco_git_config_next=$((__coco_git_config_next + 1))",
                    "  fi",
                    "  __coco_git_config_i=$((__coco_git_config_i + 1))",
                    "done",
                    "eval \"export GIT_CONFIG_KEY_${__coco_git_config_next}=\\$__coco_git_config_key\"",
                    "eval \"export GIT_CONFIG_VALUE_${__coco_git_config_next}=\"",
                    "__coco_git_config_next=$((__coco_git_config_next + 1))",
                    "eval \"export GIT_CONFIG_KEY_${__coco_git_config_next}=\\$__coco_git_config_key\"",
                    "eval \"export GIT_CONFIG_VALUE_${__coco_git_config_next}=\\$__coco_git_config_value\"",
                    "export GIT_CONFIG_COUNT=$((__coco_git_config_next + 1))",
                    "unset __coco_git_config_original_count __coco_git_config_next __coco_git_config_i __coco_git_config_key __coco_git_config_value __coco_git_config_existing_key __coco_git_config_existing_value",
                ]
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn gh_adapter_git_config_env_is_idempotent() {
        use std::process::Command;

        with_temp_config_root(|_temp| {
            let config = config_with_scope(&["github"]);
            let exports = render_tool_env(&config, "gh", "laptop").unwrap();
            let script = format!(
                r#"
GIT_CONFIG_COUNT=4
GIT_CONFIG_KEY_0=credential.https://gw.example.com.helper
GIT_CONFIG_VALUE_0='!old-helper'
GIT_CONFIG_KEY_1=core.editor
GIT_CONFIG_VALUE_1=vim
GIT_CONFIG_KEY_2=credential.https://other.example.com.helper
GIT_CONFIG_VALUE_2='!other-helper'
GIT_CONFIG_KEY_3=credential.https://gw.example.com.helper
GIT_CONFIG_VALUE_3='!older-helper'
{exports}
{exports}
printf '%s\n' \
  "$GIT_CONFIG_COUNT" \
  "$GIT_CONFIG_KEY_0" "$GIT_CONFIG_VALUE_0" \
  "$GIT_CONFIG_KEY_1" "$GIT_CONFIG_VALUE_1" \
  "$GIT_CONFIG_KEY_2" "$GIT_CONFIG_VALUE_2" \
  "$GIT_CONFIG_KEY_3" "$GIT_CONFIG_VALUE_3"
"#,
                exports = exports.join("\n")
            );

            let output = Command::new("sh").arg("-c").arg(script).output().unwrap();

            assert!(
                output.status.success(),
                "stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                concat!(
                    "4\n",
                    "core.editor\n",
                    "vim\n",
                    "credential.https://other.example.com.helper\n",
                    "!other-helper\n",
                    "credential.https://gw.example.com.helper\n",
                    "\n",
                    "credential.https://gw.example.com.helper\n",
                    "!coco git-credential laptop\n",
                )
            );
        });
    }

    #[test]
    fn opencode_env_materializes_managed_config() {
        with_temp_config_root(|temp| {
            let config = config_with_scope(&["openai", "anthropic"]);
            let exports = render_tool_env(&config, "opencode", "laptop").unwrap();

            let expected_path = temp
                .path()
                .join(".config/coco/generated/opencode/laptop/opencode.json");
            assert!(expected_path.exists());
            assert!(exports.contains(&format!(
                "export OPENCODE_CONFIG={}",
                expected_path.display()
            )));
            assert!(exports.contains(&"export OPENAI_API_KEY=ccgw_test".to_string()));
            assert!(exports.contains(&"export ANTHROPIC_API_KEY=ccgw_test".to_string()));
        });
    }

    #[test]
    fn codex_install_writes_default_config() {
        with_temp_config_root(|temp| {
            let config = config_with_scope(&["openai"]);
            let path = install_tool_file(&config, "codex", "laptop").unwrap();

            let expected = temp.path().join(".codex/config.toml");
            assert_eq!(path, expected);
            let contents = std::fs::read_to_string(path).unwrap();
            assert!(contents.contains("openai_base_url = \"https://gw.example.com/openai\""));
            assert!(contents.contains("api_key = \"ccgw_test\""));
        });
    }

    #[test]
    fn user_tools_file_overrides_builtin_adapter() {
        with_temp_config_root(|_temp| {
            let tools_path = Config::tools_path();
            std::fs::create_dir_all(tools_path.parent().unwrap()).unwrap();
            std::fs::write(
                &tools_path,
                r#"
[adapters.gh]
[[adapters.gh.env]]
key = "GH_OVERRIDE"
value = "1"
"#,
            )
            .unwrap();

            let config = config_with_scope(&["github"]);
            let exports = render_tool_env(&config, "gh", "laptop").unwrap();
            assert_eq!(exports, vec!["export GH_OVERRIDE=1"]);
        });
    }

    #[test]
    fn experimental_claude_render_is_available() {
        with_temp_config_root(|_temp| {
            let config = config_with_scope(&["anthropic"]);
            let adapter = get_tool_adapter("claude-code").unwrap();
            assert!(adapter.experimental);

            let render = render_tool_file(&config, "claude-code", "laptop").unwrap();
            assert!(render.contains("export ANTHROPIC_API_KEY=\"ccgw_test\""));
            assert!(
                render.contains("export ANTHROPIC_BASE_URL=\"https://gw.example.com/anthropic\"")
            );
        });
    }

    #[test]
    fn default_file_error_names_tool_and_required_route() {
        with_temp_config_root(|_temp| {
            let config = config_with_scope(&["github"]);
            let error = render_tool_file(&config, "codex", "laptop").unwrap_err();
            let message = error.to_string();

            assert!(message.contains("tool 'codex'"));
            assert!(message.contains("route 'openai'"));
        });
    }

    #[test]
    fn managed_path_rejects_parent_traversal() {
        with_temp_config_root(|_temp| {
            let tools_path = Config::tools_path();
            std::fs::create_dir_all(tools_path.parent().unwrap()).unwrap();
            std::fs::write(
                &tools_path,
                r#"
[adapters.bad]
[[adapters.bad.files]]
id = "escape"
managed_path = "../escape.txt"
content = "nope"

[[adapters.bad.env]]
key = "BAD_CONFIG"
value = "{{managed_file:escape}}"
"#,
            )
            .unwrap();

            let config = config_with_scope(&[]);
            let error = render_tool_env(&config, "bad", "laptop").unwrap_err();
            assert!(error
                .to_string()
                .contains("Invalid managed_path '../escape.txt'"));
        });
    }

    #[test]
    fn managed_path_rejects_absolute_paths() {
        with_temp_config_root(|_temp| {
            let tools_path = Config::tools_path();
            std::fs::create_dir_all(tools_path.parent().unwrap()).unwrap();
            std::fs::write(
                &tools_path,
                r#"
[adapters.bad]
[[adapters.bad.files]]
id = "absolute"
managed_path = "/tmp/escape.txt"
content = "nope"

[[adapters.bad.env]]
key = "BAD_CONFIG"
value = "{{managed_file:absolute}}"
"#,
            )
            .unwrap();

            let config = config_with_scope(&[]);
            let error = render_tool_env(&config, "bad", "laptop").unwrap_err();
            assert!(error
                .to_string()
                .contains("Invalid managed_path '/tmp/escape.txt'"));
        });
    }
}

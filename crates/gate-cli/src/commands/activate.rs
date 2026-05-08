use crate::config::Config;
use crate::tooling::{self, Activation, ActivationMode};
use anyhow::{bail, Result};
use std::io::IsTerminal;
use std::process::Command;

pub fn run(
    name: &str,
    eval: bool,
    describe: bool,
    tools: &[String],
    route: Option<&str>,
) -> Result<()> {
    if describe && eval {
        bail!("--describe cannot be combined with --eval");
    }

    let config = Config::load()?;
    let tool_filter = (!tools.is_empty()).then_some(tools);

    if describe {
        let activation =
            tooling::activate(&config, name, tool_filter, route, ActivationMode::Describe)?;
        print_description(name, &activation);
        return Ok(());
    }

    let activation =
        tooling::activate(&config, name, tool_filter, route, ActivationMode::Generated)?;

    if eval || !std::io::stdout().is_terminal() {
        for line in activation.shell_lines() {
            println!("{line}");
        }
        return Ok(());
    }

    launch_subshell(name, &activation)
}

fn print_description(name: &str, activation: &Activation) {
    println!("Enclave Credential Gateway activation for token '{name}':");
    for line in activation.describe_lines() {
        println!("  {line}");
    }
}

fn launch_subshell(name: &str, activation: &Activation) -> Result<()> {
    println!("Enclave Credential Gateway activated: {name}");
    if !activation.exports.is_empty() {
        println!("Exports:");
        for export in &activation.exports {
            println!("  {}", export.key);
        }
    }
    if !activation.files.is_empty() {
        println!("Files:");
        for path in &activation.files {
            println!("  {}", path.display());
        }
    }
    println!("Type 'exit' to leave.");

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut command = Command::new(shell);
    for export in &activation.exports {
        command.env(&export.key, &export.value);
    }
    let status = command.status()?;
    if !status.success() {
        bail!("Activated shell exited with status {status}");
    }
    Ok(())
}

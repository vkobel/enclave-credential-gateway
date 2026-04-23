use crate::config::Config;
use crate::tooling;
use anyhow::Result;

pub fn run_env(tool: &str, token_name: &str) -> Result<()> {
    let config = Config::load()?;
    warn_if_experimental(tool)?;
    for line in tooling::render_tool_env(&config, tool, token_name)? {
        println!("{line}");
    }
    Ok(())
}

pub fn run_render(tool: &str, token_name: &str) -> Result<()> {
    let config = Config::load()?;
    warn_if_experimental(tool)?;
    print!("{}", tooling::render_tool_file(&config, tool, token_name)?);
    Ok(())
}

pub fn run_install(tool: &str, token_name: &str) -> Result<()> {
    let config = Config::load()?;
    warn_if_experimental(tool)?;
    let path = tooling::install_tool_file(&config, tool, token_name)?;
    eprintln!("Wrote {}", path.display());
    Ok(())
}

fn warn_if_experimental(tool: &str) -> Result<()> {
    if tooling::get_tool_adapter(tool)?.experimental {
        eprintln!("Warning: tool adapter '{}' is experimental", tool);
    }
    Ok(())
}

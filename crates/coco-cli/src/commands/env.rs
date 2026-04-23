use crate::config::Config;
use crate::tooling;
use anyhow::Result;

pub fn run(name: &str, codex: bool) -> Result<()> {
    let config = Config::load()?;
    for line in tooling::render_tool_env(&config, "shell", name)? {
        println!("{line}");
    }

    if codex {
        eprintln!("Warning: `coco env --codex` is deprecated; use `coco tool install codex {name}`");
        let _ = tooling::install_tool_file(&config, "codex", name)?;
    }

    Ok(())
}

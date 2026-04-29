use crate::config::Config;
use crate::tooling;
use anyhow::{bail, Result};

pub fn run(
    name: &str,
    write: bool,
    tools: &[String],
    route: Option<&str>,
    render: Option<&str>,
) -> Result<()> {
    let config = Config::load()?;

    if let Some(render) = render {
        let (tool, file_id) = parse_render_target(render)?;
        if tooling::get_tool_adapter(tool)?.experimental {
            eprintln!("Warning: tool adapter '{}' is experimental", tool);
        }
        print!(
            "{}",
            tooling::render_tool_file_by_id(&config, tool, name, file_id)?
        );
        return Ok(());
    }

    let tool_filter = (!tools.is_empty()).then_some(tools);
    for line in tooling::activate(&config, name, tool_filter, route, write)? {
        println!("{line}");
    }

    Ok(())
}

fn parse_render_target(render: &str) -> Result<(&str, Option<&str>)> {
    if render.is_empty() {
        bail!("--render expects TOOL or TOOL:FILE");
    }
    if let Some((tool, file_id)) = render.split_once(':') {
        if tool.is_empty() || file_id.is_empty() {
            bail!("--render expects TOOL or TOOL:FILE");
        }
        Ok((tool, Some(file_id)))
    } else {
        Ok((render, None))
    }
}

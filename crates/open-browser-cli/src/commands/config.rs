//! Reading and writing `config.yaml`.

use anyhow::Result;

use crate::cli::ConfigCommand;
use crate::context::Context;

pub fn execute(context: &Context, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Show => context
            .emit(&context.config, || serde_yaml::to_string(&context.config).unwrap_or_default()),
        ConfigCommand::Path => {
            let path = context.home.config_path();
            context.emit(&path, || path.display().to_string())
        }
        ConfigCommand::Set { key, value } => {
            let mut config = context.config.clone();
            match key.as_str() {
                "session" => config.session = open_browser_core::home::validate_session_name(&value)?,
                "headless" => config.headless = parse_bool(&value)?,
                "window" => {
                    let (width, height) = value
                        .split_once(['x', 'X'])
                        .ok_or_else(|| anyhow::anyhow!("write a window size as 1280x800"))?;
                    config.window = (width.trim().parse()?, height.trim().parse()?);
                }
                "chromeArgs" | "chrome-args" => {
                    // Shell-split so `ob config set chromeArgs '--proxy-server=x --lang=en'`
                    // stores two flags rather than one flag with a space in it.
                    config.chrome_args = shell_words::split(&value)?;
                }
                "openAgentsBin" | "open-agents-bin" => config.open_agents_bin = value,
                "promptware" => config.promptware = value,
                "bind" => config.bind = value,
                other => anyhow::bail!(
                    "no setting called '{other}'. \
                     Settings: session, headless, window, chromeArgs, openAgentsBin, promptware, bind"
                ),
            }
            context.home.ensure()?;
            config.save(&context.home.config_path())?;
            context
                .emit(&config, || format!("{key} set in {}", context.home.config_path().display()))
        }
    }
}

fn parse_bool(raw: &str) -> Result<bool> {
    match raw {
        "true" | "yes" | "1" | "on" => Ok(true),
        "false" | "no" | "0" | "off" => Ok(false),
        other => anyhow::bail!("'{other}' is not true or false"),
    }
}

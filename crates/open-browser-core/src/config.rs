//! `$OPEN_BROWSER_HOME/config.yaml` — defaults, so that the common case is `ob goto x.com`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{io, Error, Result};

/// The session used when `--session` is absent.
pub const DEFAULT_SESSION: &str = "default";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub session: String,
    /// Whether a session starts headless. Off for a session a person logs in with by hand.
    pub headless: bool,
    pub window: (u32, u32),
    /// Extra Chrome flags applied to every session.
    pub chrome_args: Vec<String>,
    /// How the agent bridge invokes open-agents. Looked up on `$PATH` unless it is a path.
    pub open_agents_bin: String,
    /// The promptware an agent run uses when `--promptware` is absent.
    pub promptware: String,
    /// Where the service listens.
    pub bind: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            session: DEFAULT_SESSION.to_string(),
            headless: true,
            window: (1280, 800),
            chrome_args: Vec::new(),
            open_agents_bin: "open-agents".to_string(),
            promptware: "Browser".to_string(),
            bind: "127.0.0.1:8787".to_string(),
        }
    }
}

impl Config {
    /// Read it, treating a missing file as an empty one — `ob` must work before anything is
    /// configured.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_yaml::from_str(&text)
                .map_err(|error| Error::other(format!("{}: {error}", path.display()))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => {
                Err(Error::Io { context: format!("reading {}", path.display()), source })
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            io(std::fs::create_dir_all(parent), || format!("creating {}", parent.display()))?;
        }
        let body = serde_yaml::to_string(self)
            .map_err(|error| Error::other(format!("serialising the config: {error}")))?;
        io(std::fs::write(path, body), || format!("writing {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_config_is_the_default_rather_than_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&dir.path().join("nothing.yaml")).unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn a_partial_config_keeps_the_defaults_for_what_it_omits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "headless: false\n").unwrap();
        let config = Config::load(&path).unwrap();
        assert!(!config.headless);
        assert_eq!(config.session, DEFAULT_SESSION);
    }

    #[test]
    fn a_config_round_trips_through_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let config = Config {
            session: "work".into(),
            chrome_args: vec!["--lang=en-GB".into()],
            ..Default::default()
        };
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
    }

    #[test]
    fn a_malformed_config_names_the_file_rather_than_falling_back_silently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "headless: [not, a, bool]\n").unwrap();
        let error = Config::load(&path).unwrap_err();
        assert!(error.to_string().contains("config.yaml"), "{error}");
    }
}

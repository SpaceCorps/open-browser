//! Where open-browser keeps its state.

use std::path::{Path, PathBuf};

use crate::error::{io, Result};

/// ```text
/// $OPEN_BROWSER_HOME/            (default ~/.open-browser)
///   profiles/<session>/          Chrome's user-data-dir: cookies, logins, history
///   sessions/<session>/          captures and downloads from that session
///   automations/<name>.yaml      saved action scripts
///   runs.sqlite3                 what ran, when, and what it returned
///   config.yaml                  defaults
/// ```
///
/// `profiles/` is the reason this is not a temp directory: a session that has logged into an email
/// account is worth keeping, and is also the most sensitive thing here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Home(PathBuf);

impl Home {
    pub fn resolve(override_path: Option<&Path>) -> Result<Self> {
        if let Some(path) = override_path {
            return Ok(Self(path.to_path_buf()));
        }
        if let Some(raw) = std::env::var_os(crate::HOME_ENV) {
            let raw = PathBuf::from(raw);
            if !raw.as_os_str().is_empty() {
                return Ok(Self(raw));
            }
        }
        let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Ok(Self(base.join(".open-browser")))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.0.join("profiles")
    }

    /// Chrome's `--user-data-dir` for one session. Everything that makes a session "logged in"
    /// lives here.
    pub fn profile_dir(&self, session: &str) -> PathBuf {
        self.profiles_dir().join(session)
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.0.join("sessions")
    }

    /// Where captures and downloads land when no `--path` was given.
    pub fn session_dir(&self, session: &str) -> PathBuf {
        self.sessions_dir().join(session)
    }

    pub fn automations_dir(&self) -> PathBuf {
        self.0.join("automations")
    }

    pub fn automation_path(&self, name: &str) -> PathBuf {
        self.automations_dir().join(format!("{name}.yaml"))
    }

    pub fn database_path(&self) -> PathBuf {
        self.0.join("runs.sqlite3")
    }

    pub fn config_path(&self) -> PathBuf {
        self.0.join("config.yaml")
    }

    pub fn ensure(&self) -> Result<()> {
        for dir in
            [self.0.clone(), self.profiles_dir(), self.sessions_dir(), self.automations_dir()]
        {
            io(std::fs::create_dir_all(&dir), || format!("creating {}", dir.display()))?;
        }
        Ok(())
    }
}

/// Reject a session name that is not a plain directory name.
///
/// Session names reach the filesystem as a profile directory and arrive from an HTTP path segment,
/// so `../` in one would be a directory-traversal write.
pub fn validate_session_name(raw: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(crate::Error::other("a session name cannot be empty"));
    }
    if name.starts_with('.') || name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(crate::Error::other(format!(
            "'{raw}' is not a usable session name: it must be a plain name, not a path"
        )));
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_home_wins_and_nothing_is_created_until_ensure() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::resolve(Some(&dir.path().join("state"))).unwrap();
        assert!(!home.path().exists());
        home.ensure().unwrap();
        assert!(home.profiles_dir().is_dir());
        assert!(home.automations_dir().is_dir());
    }

    #[test]
    fn session_names_that_would_escape_the_profiles_directory_are_rejected() {
        for bad in ["../elsewhere", "a/b", "", ".hidden", "x\0y"] {
            assert!(validate_session_name(bad).is_err(), "{bad} should have been rejected");
        }
        assert_eq!(validate_session_name("  work  ").unwrap(), "work");
    }
}

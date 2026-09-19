//! The `Browser` promptware, compiled into `ob` and installed into open-agents' home.
//!
//! `ob agent run` shells out to `open-agents run --promptware=Browser`, and open-agents resolves
//! that name under *its* home, not this one. So a machine with both binaries installed still fails
//! on the first agent run unless something writes the program into `~/.open-agents/agents/Browser`.
//! `ob` ships it and does exactly that, because it is the half of the pair that knows what a
//! browser agent should be told.
//!
//! Installing never overwrites an existing `Program.md`. Once it is on disk it belongs to the
//! user — an upgrade that reset an edited program would throw away the tuning that makes their
//! agent good at their sites.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

/// The name `config.promptware` defaults to.
pub const NAME: &str = "Browser";

/// open-agents' own home variable. Read here rather than shelling out, so this works before
/// open-agents is installed.
const AGENTS_HOME_ENV: &str = "OPEN_AGENTS_HOME";

const PROGRAM: &str = include_str!("../../../agents/Browser/Program.md");

/// Where open-agents keeps its promptwares, by the same rule open-agents itself uses.
pub fn agents_home() -> PathBuf {
    if let Some(raw) = std::env::var_os(AGENTS_HOME_ENV) {
        let path = PathBuf::from(raw);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    dirs_home().join(".open-agents")
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn program_path(agents_home: &Path) -> PathBuf {
    agents_home.join("agents").join(NAME).join("Program.md")
}

/// What an install did, so the caller can say so without guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// The program was written; this is the first time.
    Created,
    /// A `Program.md` was already there and was left exactly as it was.
    Kept,
}

/// Write the promptware into `agents_home` unless its program is already there.
pub fn install(agents_home: &Path) -> Result<Installed> {
    let root = agents_home.join("agents").join(NAME);
    // Memory/ and Tools/ are created either way: open-agents lists their contents when it compiles
    // the firmware, and a promptware missing them is one whose learning loop has nowhere to write.
    for directory in [root.join("Memory"), root.join("Tools")] {
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("creating {}", directory.display()))?;
    }
    let program = root.join("Program.md");
    if program.is_file() {
        return Ok(Installed::Kept);
    }
    std::fs::write(&program, PROGRAM).with_context(|| format!("writing {}", program.display()))?;
    Ok(Installed::Created)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The program is the agent's entire briefing on how to reach the browser. If it stopped
    /// naming `ob`, an agent would be left to invent an interface that does not exist.
    #[test]
    fn the_shipped_program_tells_the_agent_to_drive_the_browser_through_this_cli() {
        assert!(PROGRAM.contains("ob --json"), "the program must show real `ob` invocations");
        assert!(
            PROGRAM.contains("ob actions list"),
            "the program must point at the registry, not a list it repeats and lets go stale"
        );
    }

    /// Every long flag the program names must be one the CLI actually accepts.
    ///
    /// The program shipped telling agents that a person should run `ob session start
    /// --no-headless`; the flag is `--headed`. Nothing caught it, because prose in a `.md` file is
    /// not compiled — and the agent would have passed that instruction on to a person as fact.
    #[test]
    fn every_flag_the_program_mentions_is_a_flag_the_cli_has() {
        // Recursive, because the flag that prompted this test is on `session start` — two levels
        // down — and a one-level walk would have passed while the program was still wrong.
        fn longs(command: &clap::Command, into: &mut std::collections::HashSet<String>) {
            into.extend(
                command.get_arguments().filter_map(|arg| arg.get_long()).map(str::to_string),
            );
            for sub in command.get_subcommands() {
                longs(sub, into);
            }
        }
        let mut known = std::collections::HashSet::new();
        longs(&crate::cli::build(), &mut known);

        for word in PROGRAM.split(|c: char| c.is_whitespace() || c == '`' || c == '=') {
            let Some(flag) = word.strip_prefix("--") else { continue };
            let flag = flag.trim_end_matches(|c: char| !c.is_alphanumeric());
            if flag.is_empty() || known.contains(flag) {
                continue;
            }
            panic!("the program tells agents about `--{flag}`, which no `ob` command accepts");
        }
    }

    /// The Reflection step lives in open-agents' firmware, but it has nothing to ask for unless the
    /// program says what is worth remembering. Without this section a browser agent rediscovers
    /// every selector on every run.
    #[test]
    fn the_shipped_program_asks_for_memory_worth_keeping() {
        assert!(PROGRAM.contains("What is worth remembering"));
        assert!(PROGRAM.contains("[[other-file]]"), "cross-references keep memory navigable");
    }

    #[test]
    fn installing_creates_the_program_and_the_two_directories_the_firmware_lists() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(install(dir.path()).unwrap(), Installed::Created);
        let root = dir.path().join("agents").join(NAME);
        assert_eq!(std::fs::read_to_string(root.join("Program.md")).unwrap(), PROGRAM);
        assert!(root.join("Memory").is_dir());
        assert!(root.join("Tools").is_dir());
    }

    #[test]
    fn installing_over_an_edited_program_keeps_the_users_version() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path()).unwrap();
        let program = program_path(dir.path());
        std::fs::write(&program, "# Mine\n").unwrap();
        assert_eq!(install(dir.path()).unwrap(), Installed::Kept);
        assert_eq!(std::fs::read_to_string(&program).unwrap(), "# Mine\n");
    }

    #[test]
    fn the_agents_home_follows_open_agents_own_variable() {
        std::env::set_var(AGENTS_HOME_ENV, "/tmp/somewhere-else");
        assert_eq!(agents_home(), PathBuf::from("/tmp/somewhere-else"));
        std::env::remove_var(AGENTS_HOME_ENV);
        assert!(agents_home().ends_with(".open-agents"));
    }
}

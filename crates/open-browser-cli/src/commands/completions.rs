//! Shell completions, generated from the same command tree the parser uses.
//!
//! Because the tree includes the registry-generated action subcommands, completions cover them
//! too: tab-completing `ob scr` offers `screenshot` and `scroll` without anyone listing them here.

use clap_complete::{generate, Shell};

use crate::cli::CompletionShell;

pub fn execute(shell: CompletionShell) {
    let shell = match shell {
        CompletionShell::Bash => Shell::Bash,
        CompletionShell::Zsh => Shell::Zsh,
        CompletionShell::Fish => Shell::Fish,
        CompletionShell::Elvish => Shell::Elvish,
        CompletionShell::PowerShell => Shell::PowerShell,
    };
    let mut command = crate::cli::build();
    generate(shell, &mut command, "ob", &mut std::io::stdout());
}

//! `ob actions list` and `ob actions show`: the registry as data.
//!
//! This is also the agent's discovery path — `ob actions list --json` is what a promptware points
//! an agent at when it needs to know what it can do.

use anyhow::Result;
use open_browser_core::actions::{find, registry, ActionSpec, ParamKind};

use crate::cli::ActionsCommand;
use crate::context::Context;

pub fn execute(context: &Context, command: ActionsCommand) -> Result<()> {
    match command {
        ActionsCommand::List { group } => {
            let specs: Vec<&ActionSpec> = registry()
                .iter()
                .filter(|spec| group.as_deref().is_none_or(|group| spec.group == group))
                .collect();
            if specs.is_empty() {
                let groups = groups().join(", ");
                anyhow::bail!("no actions in that group. Groups: {groups}");
            }
            context.emit(&specs, || human_list(&specs))
        }
        ActionsCommand::Show { id } => {
            let spec = find(&id).ok_or_else(|| {
                anyhow::anyhow!(
                    "no action called '{id}'. `ob actions list` shows all {} of them",
                    registry().len()
                )
            })?;
            context.emit(&spec, || human_show(spec))
        }
    }
}

/// Every group name, once each, in the order the registry first mentions them.
///
/// `dedup` alone would be wrong: the registry is ordered by how a person reads it — `upload` sits
/// with the other interactions even though `download` came before it — so a group's actions are
/// not necessarily contiguous, and two runs of "read" would survive as two entries.
fn groups() -> Vec<&'static str> {
    let mut groups: Vec<&'static str> = Vec::new();
    for spec in registry() {
        if !groups.contains(&spec.group) {
            groups.push(spec.group);
        }
    }
    groups
}

/// The listing a person reads, one heading per group.
///
/// Collected per group rather than emitted as the registry is walked: an action is placed where it
/// belongs in reading order, not next to its group-mates, so `read` appears at three separate
/// points in the registry and a break-on-change loop would print that heading three times.
fn human_list(specs: &[&ActionSpec]) -> String {
    let width = specs.iter().map(|spec| spec.id.len()).max().unwrap_or(0);
    let mut lines = Vec::new();
    for group in groups() {
        let members: Vec<&&ActionSpec> = specs.iter().filter(|spec| spec.group == group).collect();
        if members.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("{group}:"));
        for spec in members {
            lines.push(format!("  {:width$}  {}", spec.id, spec.summary));
        }
    }
    lines.join("\n")
}

fn human_show(spec: &ActionSpec) -> String {
    let mut lines = vec![
        format!("{}  {}", spec.id, spec.summary),
        format!(
            "group: {}   {}",
            spec.group,
            if spec.mutates { "changes the page" } else { "read-only" }
        ),
    ];
    if spec.params.is_empty() {
        lines.push("takes no parameters".into());
    } else {
        lines.push("parameters:".into());
        let width = spec.params.iter().map(|param| param.name.len()).max().unwrap_or(0);
        for param in spec.params {
            let kind = match param.kind {
                ParamKind::Choice(choices) => choices.join("|"),
                ParamKind::Flag => "flag".into(),
                ParamKind::Selector => "selector".into(),
                ParamKind::Url => "url".into(),
                ParamKind::Path => "path".into(),
                ParamKind::Number => "number".into(),
                ParamKind::Text => "text".into(),
            };
            lines.push(format!(
                "  {:width$}  {:<12} {}{}{}",
                param.name,
                kind,
                if param.required { "(required) " } else { "" },
                if param.repeatable { "(repeatable) " } else { "" },
                param.help
            ));
        }
    }
    lines.push(format!("example:\n  {}", spec.example));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is ordered for a reader, not by group, so `read` and `interact` each appear at
    /// several points in it. A formatter that starts a new heading whenever the group changes
    /// prints "read:" three times and splits those actions across three blocks.
    #[test]
    fn the_listing_names_each_group_once() {
        let specs: Vec<&ActionSpec> = registry().iter().collect();
        let listing = human_list(&specs);
        for group in groups() {
            let heading = format!("{group}:");
            let count = listing.lines().filter(|line| *line == heading).count();
            assert_eq!(count, 1, "'{heading}' appears {count} times:\n{listing}");
        }
    }

    /// Whatever the grouping does to the order, nothing may fall out of the listing — an action a
    /// person cannot find is one they will not know to ask an agent for.
    #[test]
    fn the_listing_carries_every_action() {
        let specs: Vec<&ActionSpec> = registry().iter().collect();
        let listing = human_list(&specs);
        for spec in registry() {
            assert!(
                listing.lines().any(|line| line.trim_start().starts_with(spec.id)),
                "'{}' is missing from the listing",
                spec.id
            );
        }
    }

    /// `ob actions list --group=read` shows that group's heading and nothing else's.
    #[test]
    fn filtering_to_one_group_leaves_the_others_out() {
        let specs: Vec<&ActionSpec> =
            registry().iter().filter(|spec| spec.group == "read").collect();
        let listing = human_list(&specs);
        assert!(listing.starts_with("read:"), "{listing}");
        for group in groups().into_iter().filter(|group| *group != "read") {
            assert!(!listing.contains(&format!("{group}:")), "{group} leaked in:\n{listing}");
        }
    }
}

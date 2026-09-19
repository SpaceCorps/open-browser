# Working on open-browser

The invariants that are not obvious from the code, for whoever — or whatever — edits it next.

## Build and check

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test                                      # 73 tests, fast, no browser and no network

pnpm install
pnpm --filter @spacecorps/open-browser-web check   # oxlint (type-aware) + oxfmt + tsc
pnpm --filter @spacecorps/open-browser-web test    # needs a built `ob`; see below
pnpm --filter @spacecorps/open-browser-web build
```

CI runs exactly these. No test needs Chrome installed or a network connection — the engine tests
cover parsing and shaping, not driving, which is deliberate: a suite that needs a browser is a
suite that does not run.

`rustfmt.toml` sets `use_small_heuristics = "Max"`. The code is written dense on purpose, and the
default heuristics explode a 90-character struct literal across five lines.

## The one rule

**Every browser capability has a CLI command.** An agent's only interface is a shell, so a
capability reachable from the HTTP API or the web UI but not from `ob` is a capability agents do
not have. This is not a style preference; it is what the project is.

What enforces it is that all four surfaces are generated from one table:

```
crates/open-browser-core/src/actions/spec.rs      ActionSpec: id, group, params, example
                    |
   +----------------+--------------------+---------------------+
   |                |                    |                     |
cli/src/actions.rs  server/src/state.rs  Action::parse         apps/web Actions tab
clap subcommands    POST /api/actions/*  the one executor      GET /api/actions
```

Adding an action means adding a `ActionSpec` and an `Action` variant, and then it exists everywhere.
Do not add a route, a flag or a UI control that reaches past this. `every_action_has_a_cli_command`
and `every_management_command_avoids_an_action_id` fail if you try.

## Things that bit us

**Serde casing is load-bearing.** `SessionRecord`, `ActionOutcome`, `RunRecord` and `Config` are
`camelCase`; `ParamKind` is `kebab-case` and its `Choice` variant serialises as `{"choice": [...]}`
while every other variant is a bare string; `Automation` and `Step` have **no** rename and
`Step.params` is `#[serde(flatten)]`. `apps/web/src/api/types.ts` mirrors all of this by hand.
Changing an attribute without changing that file produces `undefined` in the UI, not a type error.

**Pin the session's tab.** A freshly connected CDP handler has not finished discovering targets, so
an immediate `pages()` is empty and a naive caller opens a *second* tab; and targets live in a
`HashMap`, so "the first page" is whichever the hash order gives. A session therefore records the
target id it started with and every later command resolves exactly that. Without it `ob goto`
navigates one tab and `ob text` reads another, blank one.

**The browser must outlive the command.** `ob session start` spawns Chrome detached, in its own
process group, and `mem::forget`s the child. chromiumoxide's own `Browser::launch` sets
`kill_on_drop`, which is right for a one-shot and exactly wrong for a session.

**`property` and `attribute` are not the same thing.** `ob attribute input value` has to read the
DOM *property* — the attribute holds what the HTML said, not what the user typed.

**The registry is ordered for a reader, not by group.** `read` appears at three separate points in
it. Anything that formats by group must collect per group rather than break on change, or you get
three "read:" headings. `the_listing_names_each_group_once` guards this.

**Tauri's stable line is 2.x, and crates.io's "latest" is not.** `cargo info tauri` reports
3.0.0-alpha.1. The workspace pins `tauri = "2.11"`, `tauri-build = "2.6"`, `tauri-plugin-opener =
"2.5"` explicitly.

## The desktop shell

`apps/desktop/src-tauri/src/lib.rs` starts the service in-process on `127.0.0.1:0` and injects the
address into the webview as `window.__OPEN_BROWSER_API__`. That injection has to happen before the
page loads, so the port must be known before the window is created — which is why
`open_browser_server` splits `bind()` (claims the port, returns `Serving` with its address) from
`Serving::run()` (never returns). The window is built in `setup` rather than declared in
`tauri.conf.json`, because a config window is created before that hook runs and would load without
the script. `"windows": []` in the config is therefore deliberate.

## The web UI

`@spacecorps/components` is private and this repository is public, so `apps/web` carries a local
`cn()` and a token subset under the design system's *identical* names rather than depending on it.
See `apps/web/README.md`.

`src/lib/command.ts` reimplements `ob`'s positional-argument placement in TypeScript so the UI can
show the equivalent command. A drift there would print a command that does not do what the panel
did, so `tests/command.test.ts` generates its fixtures by running the real binary:

```bash
cargo build -p open-browser-cli --bin ob     # the test shells out to it
pnpm --filter @spacecorps/open-browser-web test
```

## Agents and memory

This repo never calls a model. `ob agent run` builds an argv for `open-agents` and supervises the
process; providers, keys and streaming all live there. The bundled promptware is
`agents/Browser/Program.md`, compiled into the binary and written into open-agents' home by
`ob agent install` — which will not overwrite an edited copy.

The promptware tells agents to stop at a login wall rather than try credentials they were not
given, and not to work around a CAPTCHA. It also bounds what may be written to memory: selectors
and the shape of a flow, never a credential and never page content. Both are prose in that file,
which is the point — you change how agents behave by editing it. `the_shipped_program_*` tests
assert the parts that other code depends on.

## Layout

```
crates/open-browser-core/
  actions/spec.rs      the registry: one ActionSpec per capability
  actions/mod.rs       Action, and the parse the CLI and the API share
  engine/chrome.rs     finding Chrome, launching it detached, attaching
  engine/execute.rs    one match arm per action, against CDP
  session.rs           the session registry and process control
  runs.rs              SQLite: what ran, when, what it returned
  automation.rs        YAML scripts over the same actions
  agents.rs            the argv handed to open-agents
crates/open-browser-cli/
  cli.rs               the management commands (enum)
  actions.rs           the action commands (generated from the registry)
  commands/            one module per management command
crates/open-browser-server/
  state.rs             handlers; routes come from the registry
  ws.rs                /api/events — snapshot then diffs
apps/web/              the browser UI
apps/desktop/          the Tauri 2 shell
agents/Browser/        the promptware
```

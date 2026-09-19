# open-browser

Give an agent a real Chrome session and one shell command per thing a browser can do.

```bash
cargo install --git https://github.com/SpaceCorps/open-browser open-browser-cli
ob doctor

ob session start --headed        # log into whatever you need, once, by hand
ob agent run "find the newest invoice in my email and save it as a PDF"
```

## What it is

**One command per capability.** Twenty-four of them — `ob goto`, `ob click`, `ob type`,
`ob wait-for`, `ob text`, `ob screenshot`, `ob download` — and `ob actions list` is the complete
set. That is the constraint the whole project is built around: *anything an agent can do in a
browser has a CLI command*, because the shell is the only interface the agent has.

**A session is a persistent Chrome profile.** Cookies and logins survive between commands, between
runs, and across a reboot. You log into your email once, in a visible window, and every agent run
after that is already logged in.

**An agent is open-agents wearing a promptware.** `ob agent run` hands the task to
[open-agents](https://github.com/SpaceCorps/open-agents) with the bundled `Browser` promptware,
which tells the agent to drive this CLI and to write down what it learned about each site. Next run
on the same site, that memory is in its prompt — the selector for the compose button, the shape of
the search results — so it does not rediscover it. **This repository never calls a model.** It
shells out to `open-agents`, which is where every provider and API key lives.

**Fleets.** `ob agent run -n 12 "…"` starts twelve agents in twelve sessions, each with its own
browser profile, and `ob runs` follows them.

## The three surfaces

An action is declared once, in
[`crates/open-browser-core/src/actions/`](crates/open-browser-core/src/actions), and that one
declaration produces all of:

| | |
| --- | --- |
| `ob click 'button.go'` | the CLI subcommand, its flags and its help |
| `POST /api/actions/click` | the HTTP route, parsed by the identical function |
| the agent's tool listing | `ob actions list --json`, which the promptware embeds |
| the Actions tab | the web UI builds its palette and forms from `GET /api/actions` |

Nothing anywhere enumerates actions by hand. Adding one to the registry makes it appear in all four
places, which is the only way the "everything has a CLI command" claim can stay true as the project
grows rather than being a thing someone has to remember.

## Actions

```
navigate   goto  back  forward  reload
interact   click  type  press  select  check  hover  scroll  wait-for  upload
read       text  html  attribute  links  eval  url  title
capture    screenshot  pdf  download
session    cookies
```

`ob actions show click` prints one action's parameters and an example. `--json` on anything gives
you the machine-readable form, which is what agents parse.

## Sessions

```bash
ob session start work --headed   # a visible window, to log in by hand
ob session list
ob --session work goto mail.example.com
ob session stop work
```

The profile lives in `~/.open-browser/profiles/<session>/`. It holds real cookies and real
logged-in sessions, it is in `.gitignore`, and it is the most sensitive thing this project touches.

## Automations

A saved sequence of the same actions, in YAML, with inputs:

```bash
ob automation new invoices
ob automation run invoices -i month=2026-09
```

A step goes through the identical `Action::parse` a CLI command does, so an automation cannot reach
a capability the shell lacks.

## The service and the UI

```bash
ob serve                                    # the API on 127.0.0.1:8787
ob serve --ui apps/web/dist                 # …and the built web UI with it
```

The UI is a client of that API and nothing more. Every panel that performs something also shows the
`ob` command that would do the same thing — that is how the project keeps itself honest about the
CLI being complete. See [`apps/web/README.md`](apps/web/README.md).

There is also a desktop app — the same UI and the same service in one window, with the service
bound to a random loopback port that only that window knows:

```bash
pnpm install
pnpm tauri dev
```

## Layout

```
crates/open-browser-core/    the registry, the CDP engine, sessions, runs, automations
crates/open-browser-cli/     `ob` — commands generated from the registry
crates/open-browser-server/  the HTTP API and the /api/events socket
apps/web/                    the browser UI (React, Vite+, Tailwind v4)
apps/desktop/                the Tauri 2 shell around both
agents/Browser/              the promptware `ob agent install` writes into open-agents' home
```

## Requirements

Chrome or Chromium (`ob doctor` looks for it and says where), Rust 1.85+, and
[open-agents](https://github.com/SpaceCorps/open-agents) for `ob agent`. Node and pnpm only if you
are building the UI.

## Safety

Agents are told to stop at a login wall rather than try credentials they were not given, and not to
work around a CAPTCHA. Memory holds selectors and the shape of a flow, never credentials and never
page content. Browser profiles are never committed.

## Licence

FSL-1.1-ALv2. See [LICENSE](LICENSE).

# open-browser web UI

The browser UI for the open-browser service. It is a client of `/api` and nothing more — every
button here posts to an endpoint that `ob` also reaches from the shell.

```sh
ob serve                 # the service, on 127.0.0.1:8787
pnpm dev                 # this UI, on 127.0.0.1:5173, proxying /api to it
```

`pnpm build` writes `dist/`, which two things then serve: `ob serve --ui apps/web/dist`, and the
desktop app via `frontendDist` in `apps/desktop/src-tauri/tauri.conf.json`.

## What it is allowed to be

**No capability the CLI lacks.** The project's claim is that anything an agent can do in a browser
has a shell command, and an agent only has the shell. A control here that reached past the API —
a button wired to some UI-only path — would quietly break that. So every surface that performs
something also renders the equivalent `ob` invocation through `<CommandLine>`; if a panel cannot
show you the command, it should not exist.

**Nothing enumerates actions.** `GET /api/actions` returns the registry — ids, groups, parameters,
whether the action mutates the page — and the Actions tab builds its palette and its forms from
that. Adding an action to `crates/open-browser-core/src/actions/` makes it appear here with no
edit to this package, which is the same property the CLI and the agent tool listing have.

**The command preview must be true.** `src/lib/command.ts` reimplements `ob`'s argument placement
in TypeScript, and a drifting reimplementation would print a command that does not do what the
panel just did. `tests/command.test.ts` therefore builds its fixtures by running the real binary
(`cargo run -p open-browser-cli --bin ob -- --json actions list`) rather than from a checked-in
copy of the registry, so the two cannot separate silently.

## The design system

`@spacecorps/components` is a private package and this repository is public, so depending on it
would make the repo unbuildable for anyone outside the org. Instead `src/lib/cn.ts` and
`src/index.css` carry a small local copy under the _identical_ names — `--background`, `--border`,
`--muted-foreground`, `--radius`, the same `cn()` — and `src/components/primitives.tsx` implements
the handful of elements this UI needs against them. A component lifted from the design system into
this tree keeps its classes and renders correctly, and swapping the real package back in later is
a per-import change rather than a restyle.

## Layout

| Path                 |                                                                                |
| -------------------- | ------------------------------------------------------------------------------ |
| `src/api/types.ts`   | The wire types, matching the server's serde attributes exactly                 |
| `src/api/client.ts`  | `fetch` wrapper, typed endpoints, and the `/api/events` socket with backoff    |
| `src/hooks.ts`       | `useResource`, and `useRuns`, which merges the socket's snapshot/update frames |
| `src/lib/command.ts` | The `ob` command a panel would run                                             |
| `src/views/`         | One file per tab: Actions, Agents, Runs, Automations, Sessions                 |

## Checks

```sh
pnpm check       # oxlint (type-aware) + oxfmt + tsc --noEmit
pnpm test        # needs a built `ob`; the registry fixtures come from it
pnpm build
```

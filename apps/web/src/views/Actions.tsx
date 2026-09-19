import { Loader2, Play, Search } from "lucide-react";
import { useMemo, useState } from "react";
import { api, ApiError } from "@/api/client";
import { choices, type ActionOutcome, type ActionSpec } from "@/api/types";
import { CommandLine } from "@/components/CommandLine";
import { Badge, Button, Card, Empty, Field, Input, Select } from "@/components/primitives";
import { useActions } from "@/hooks";
import { cliCommand, emptyParams } from "@/lib/command";
import { cn } from "@/lib/cn";

/** The action palette: every browser capability this build has, from the registry.
 *
 * Nothing here enumerates actions. The list, each form, and the command preview are all generated
 * from `/api/actions`, which serialises the same Rust table `ob --help` is built from. An action
 * added to the registry appears here with no change to this file. */
export function Actions({ session }: { session: string }) {
  const { data: specs, error, loading } = useActions();
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<string | null>(null);

  const groups = useMemo(() => {
    const filtered = (specs ?? []).filter((spec) => {
      const needle = query.trim().toLowerCase();
      if (!needle) return true;
      return (
        spec.id.includes(needle) ||
        spec.summary.toLowerCase().includes(needle) ||
        spec.group.includes(needle)
      );
    });
    const byGroup = new Map<string, ActionSpec[]>();
    for (const spec of filtered) {
      const list = byGroup.get(spec.group) ?? [];
      list.push(spec);
      byGroup.set(spec.group, list);
    }
    return [...byGroup.entries()];
  }, [specs, query]);

  const active = specs?.find((spec) => spec.id === selected) ?? null;

  if (error) return <Empty>{error}</Empty>;
  if (loading) return <Empty>Loading actions…</Empty>;

  return (
    <div className="grid h-full grid-cols-[minmax(230px,300px)_1fr] gap-4 overflow-hidden">
      <Card className="flex min-h-0 flex-col overflow-hidden">
        <div className="relative border-b p-2">
          <Search className="text-muted-foreground pointer-events-none absolute top-1/2 left-4 size-3.5 -translate-y-1/2" />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={`Search ${specs?.length ?? 0} actions`}
            className="h-8 pl-8 text-xs"
          />
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-1">
          {groups.length === 0 ? <Empty>Nothing matches.</Empty> : null}
          {groups.map(([group, list]) => (
            <div key={group} className="mb-2">
              <p className="text-muted-foreground px-2 py-1 text-[11px] font-semibold tracking-wide uppercase">
                {group}
              </p>
              {list.map((spec) => (
                <button
                  key={spec.id}
                  type="button"
                  onClick={() => setSelected(spec.id)}
                  className={cn(
                    "flex w-full flex-col items-start gap-0.5 rounded-md px-2 py-1.5 text-left transition-colors",
                    spec.id === selected
                      ? "bg-accent text-accent-foreground"
                      : "hover:bg-accent/50",
                  )}
                >
                  <span className="font-mono text-xs font-medium">{spec.id}</span>
                  <span className="text-muted-foreground text-[11px]">{spec.summary}</span>
                </button>
              ))}
            </div>
          ))}
        </div>
      </Card>

      {active ? (
        <ActionForm key={active.id} spec={active} session={session} />
      ) : (
        <Card className="flex items-center justify-center">
          <Empty>Pick an action. Each one is a command you can also run in a shell.</Empty>
        </Card>
      )}
    </div>
  );
}

function ActionForm({ spec, session }: { spec: ActionSpec; session: string }) {
  const [params, setParams] = useState<Record<string, string>>(() => emptyParams(spec));
  const [outcome, setOutcome] = useState<ActionOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  const missing = spec.params.filter((param) => param.required && !params[param.name]);

  const run = () => {
    setRunning(true);
    setError(null);
    // Empty values are dropped rather than sent as "": the core distinguishes an absent optional
    // parameter from an empty one, and sending "" would defeat its defaults.
    const filled = Object.fromEntries(Object.entries(params).filter(([, value]) => value !== ""));
    api
      .runAction(spec.id, filled, session)
      .then(setOutcome)
      .catch((cause: unknown) => {
        setOutcome(null);
        setError(cause instanceof ApiError ? cause.message : String(cause));
      })
      .finally(() => setRunning(false));
  };

  return (
    <Card className="flex min-h-0 flex-col overflow-hidden">
      <header className="flex items-start justify-between gap-3 border-b p-3">
        <div>
          <h2 className="flex items-center gap-2 font-mono text-sm font-semibold">
            {spec.id}
            {spec.mutates ? (
              <Badge tone="warning">changes the page</Badge>
            ) : (
              <Badge>read-only</Badge>
            )}
          </h2>
          <p className="text-muted-foreground mt-0.5 text-xs">{spec.summary}</p>
        </div>
        <Button variant="primary" onClick={run} disabled={running || missing.length > 0}>
          {running ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
          Run
        </Button>
      </header>

      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-3">
        <CommandLine command={cliCommand(spec, params, session)} />

        {spec.params.length === 0 ? (
          <p className="text-muted-foreground text-xs">This action takes no parameters.</p>
        ) : (
          <div className="grid gap-3 sm:grid-cols-2">
            {spec.params.map((param) => {
              const options = choices(param.kind);
              return (
                <Field
                  key={param.name}
                  label={
                    <span className="flex items-center gap-1.5">
                      <span className="font-mono">{param.name}</span>
                      {param.required ? <span className="text-destructive">*</span> : null}
                      {param.repeatable ? <Badge>one per line</Badge> : null}
                    </span>
                  }
                  hint={param.help}
                >
                  {param.kind === "flag" ? (
                    <label className="flex h-9 items-center gap-2 text-xs">
                      <input
                        type="checkbox"
                        checked={params[param.name] === "true"}
                        onChange={(event) =>
                          setParams((current) => ({
                            ...current,
                            [param.name]: event.target.checked ? "true" : "",
                          }))
                        }
                      />
                      <span className="text-muted-foreground">off unless ticked</span>
                    </label>
                  ) : options ? (
                    <Select
                      value={params[param.name] ?? ""}
                      onChange={(event) =>
                        setParams((current) => ({ ...current, [param.name]: event.target.value }))
                      }
                    >
                      <option value="">default ({options[0]})</option>
                      {options.map((choice) => (
                        <option key={choice} value={choice}>
                          {choice}
                        </option>
                      ))}
                    </Select>
                  ) : (
                    <Input
                      value={params[param.name] ?? ""}
                      onChange={(event) =>
                        setParams((current) => ({ ...current, [param.name]: event.target.value }))
                      }
                      placeholder={param.kind === "selector" ? "[data-testid=…]" : ""}
                      className="font-mono text-xs"
                    />
                  )}
                </Field>
              );
            })}
          </div>
        )}

        <p className="text-muted-foreground font-mono text-[11px]">Example: {spec.example}</p>

        {error ? (
          <div className="border-destructive/40 bg-destructive/10 text-destructive rounded-md border p-2 text-xs">
            {error}
          </div>
        ) : null}

        {outcome ? (
          <div className="space-y-2">
            <p className="text-xs">
              <span className="text-success font-medium">{outcome.summary}</span>
              <span className="text-muted-foreground"> · {outcome.durationMs} ms</span>
            </p>
            {outcome.artifact ? (
              <p className="text-muted-foreground font-mono text-[11px]">
                wrote {outcome.artifact}
              </p>
            ) : null}
            {outcome.value === null ? null : (
              <pre className="bg-muted/60 max-h-72 overflow-auto rounded-md border p-2 font-mono text-[11px]">
                {typeof outcome.value === "string"
                  ? outcome.value
                  : JSON.stringify(outcome.value, null, 2)}
              </pre>
            )}
          </div>
        ) : null}
      </div>
    </Card>
  );
}

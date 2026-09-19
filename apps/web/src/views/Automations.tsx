import { Loader2, Play } from "lucide-react";
import { useState } from "react";
import { api, ApiError } from "@/api/client";
import type { Automation } from "@/api/types";
import { CommandLine } from "@/components/CommandLine";
import { Badge, Button, Card, Empty, Field, Input } from "@/components/primitives";
import { useResource } from "@/hooks";
import { cn } from "@/lib/cn";

/** Saved action scripts. A YAML file per automation under `$OPEN_BROWSER_HOME/automations`.
 *
 * Editing is left to `ob automation new` and a text editor: these files are the durable, reviewable
 * form of a browser task, and a form-based editor here would be a second way to write them that
 * has to be kept in step with the schema. */
export function Automations({ session }: { session: string }) {
  const { data: names, error, loading } = useResource<string[]>(() => api.automations(), []);
  const [selected, setSelected] = useState<string | null>(null);

  if (error) return <Empty>{error}</Empty>;
  if (loading) return <Empty>Loading automations…</Empty>;

  return (
    <div className="grid h-full grid-cols-[minmax(200px,260px)_1fr] gap-4 overflow-hidden">
      <Card className="flex min-h-0 flex-col overflow-hidden">
        <header className="border-b px-3 py-2">
          <h2 className="text-sm font-semibold">Automations</h2>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto p-1">
          {(names ?? []).length === 0 ? (
            <Empty>
              None saved. <code className="font-mono">ob automation new &lt;name&gt;</code> writes a
              starter.
            </Empty>
          ) : null}
          {(names ?? []).map((name) => (
            <button
              key={name}
              type="button"
              onClick={() => setSelected(name)}
              className={cn(
                "w-full rounded-md px-2 py-1.5 text-left font-mono text-xs transition-colors",
                name === selected ? "bg-accent text-accent-foreground" : "hover:bg-accent/50",
              )}
            >
              {name}
            </button>
          ))}
        </div>
      </Card>

      {selected ? (
        <AutomationDetail key={selected} name={selected} session={session} />
      ) : (
        <Card className="flex items-center justify-center">
          <Empty>Pick an automation.</Empty>
        </Card>
      )}
    </div>
  );
}

function AutomationDetail({ name, session }: { name: string; session: string }) {
  const { data, error } = useResource<Automation>(() => api.automation(name), [name]);
  const [inputs, setInputs] = useState<Record<string, string>>({});
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; steps: unknown[] } | null>(null);
  const [runError, setRunError] = useState<string | null>(null);

  const run = () => {
    setRunning(true);
    setRunError(null);
    api
      .runAutomation(name, inputs, session)
      .then(setResult)
      .catch((cause: unknown) =>
        setRunError(cause instanceof ApiError ? cause.message : String(cause)),
      )
      .finally(() => setRunning(false));
  };

  if (error)
    return (
      <Card className="p-3 text-xs">
        <Empty>{error}</Empty>
      </Card>
    );
  if (!data)
    return (
      <Card className="p-3">
        <Empty>Loading…</Empty>
      </Card>
    );

  const declared = { ...data.inputs };
  const command = [
    "ob",
    "--session",
    session,
    "automation",
    "run",
    name,
    ...Object.entries(inputs)
      .filter(([, value]) => value !== "")
      .map(([key, value]) => `-i ${key}=${/^[\w@%+=:,./-]+$/.test(value) ? value : `'${value}'`}`),
  ].join(" ");

  return (
    <Card className="flex min-h-0 flex-col overflow-hidden">
      <header className="flex items-start justify-between gap-3 border-b p-3">
        <div>
          <h2 className="font-mono text-sm font-semibold">{data.name}</h2>
          {data.description ? (
            <p className="text-muted-foreground mt-0.5 text-xs">{data.description}</p>
          ) : null}
        </div>
        <Button variant="primary" onClick={run} disabled={running}>
          {running ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
          Run
        </Button>
      </header>

      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-3">
        <CommandLine command={command} />

        {Object.keys(declared).length > 0 ? (
          <div className="grid gap-3 sm:grid-cols-2">
            {Object.entries(declared).map(([key, fallback]) => (
              <Field
                key={key}
                label={<span className="font-mono">{key}</span>}
                hint={`default: ${fallback}`}
              >
                <Input
                  value={inputs[key] ?? ""}
                  onChange={(event) =>
                    setInputs((current) => ({ ...current, [key]: event.target.value }))
                  }
                  placeholder={fallback}
                  className="font-mono text-xs"
                />
              </Field>
            ))}
          </div>
        ) : null}

        <section>
          <h3 className="mb-1 text-xs font-semibold">Steps</h3>
          <ol className="space-y-1">
            {data.steps.map((step, index) => {
              const { action, optional, ...params } = step;
              return (
                <li
                  key={index}
                  className="bg-muted/40 rounded-md border px-2 py-1.5 font-mono text-[11px]"
                >
                  <span className="text-muted-foreground mr-2">{index + 1}.</span>
                  <span className="font-medium">{action}</span>
                  {Object.entries(params).map(([key, value]) => (
                    <span key={key} className="text-muted-foreground ml-2">
                      {key}={String(value)}
                    </span>
                  ))}
                  {optional ? <Badge className="ml-2">optional</Badge> : null}
                </li>
              );
            })}
          </ol>
        </section>

        {runError ? (
          <div className="border-destructive/40 bg-destructive/10 text-destructive rounded-md border p-2 text-xs">
            {runError}
          </div>
        ) : null}

        {result ? (
          <section>
            <h3 className="mb-1 flex items-center gap-2 text-xs font-semibold">
              Result{" "}
              <Badge tone={result.ok ? "success" : "destructive"}>
                {result.ok ? "ok" : "failed"}
              </Badge>
            </h3>
            <pre className="bg-muted/60 max-h-72 overflow-auto rounded-md border p-2 font-mono text-[11px]">
              {JSON.stringify(result.steps, null, 2)}
            </pre>
          </section>
        ) : null}
      </div>
    </Card>
  );
}

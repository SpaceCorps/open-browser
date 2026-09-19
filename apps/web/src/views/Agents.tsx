import { Bot, Loader2, Rocket } from "lucide-react";
import { useState } from "react";
import { api, ApiError } from "@/api/client";
import type { LaunchedAgent } from "@/api/types";
import { CommandLine } from "@/components/CommandLine";
import { Button, Card, Field, Input, Textarea } from "@/components/primitives";
import { useConfig, useFlash } from "@/hooks";

/** Fire off agents.
 *
 * The service never talks to a model. It hands the task to `open-agents run`, whose promptware
 * tells the agent to drive the browser by shelling out to `ob` — which is why the command preview
 * below is the whole story and not a summary of one. `count` gives each agent its own session, so
 * a fleet does not fight over one browser. */
export function Agents({ session, onLaunched }: { session: string; onLaunched: () => void }) {
  const { data: config } = useConfig();
  const [task, setTask] = useState("");
  const [count, setCount] = useState(1);
  const [provider, setProvider] = useState("");
  const [model, setModel] = useState("");
  const [effort, setEffort] = useState("");
  const [timeout, setTimeoutSeconds] = useState("");
  const [launching, setLaunching] = useState(false);
  const [launched, setLaunched] = useState<LaunchedAgent[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [flash, setFlash] = useFlash();

  const quote = (value: string) =>
    /^[\w@%+=:,./-]+$/.test(value) ? value : `'${value.replace(/'/g, `'\\''`)}'`;
  const preview = [
    "ob",
    "--session",
    quote(session),
    "agent",
    "run",
    count > 1 ? `-n ${count}` : "",
    provider ? `--provider=${quote(provider)}` : "",
    model ? `--model=${quote(model)}` : "",
    effort ? `--effort=${quote(effort)}` : "",
    timeout ? `--timeout=${timeout}` : "",
    "--",
    quote(task || "…"),
  ]
    .filter(Boolean)
    .join(" ");

  const launch = () => {
    setLaunching(true);
    setError(null);
    api
      .runAgents({
        task,
        session,
        count,
        provider: provider || undefined,
        model: model || undefined,
        effort: effort || undefined,
        timeoutSeconds: timeout ? Number(timeout) : undefined,
      })
      .then((agents) => {
        setLaunched(agents);
        setFlash(`${agents.length} agent${agents.length === 1 ? "" : "s"} running`);
        onLaunched();
      })
      .catch((cause: unknown) =>
        setError(cause instanceof ApiError ? cause.message : String(cause)),
      )
      .finally(() => setLaunching(false));
  };

  return (
    <div className="grid h-full grid-cols-1 gap-4 overflow-y-auto lg:grid-cols-[1fr_minmax(260px,340px)]">
      <Card className="flex flex-col gap-3 p-3">
        <h2 className="flex items-center gap-2 text-sm font-semibold">
          <Bot className="size-4" aria-hidden /> Launch agents
        </h2>

        <Field
          label="Task"
          hint="Plain prose. The agent reads the page before acting and writes what it learned back to its memory."
        >
          <Textarea
            rows={5}
            value={task}
            onChange={(event) => setTask(event.target.value)}
            placeholder="Open my inbox, find anything from the landlord this week, and summarise it."
          />
        </Field>

        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <Field label="Agents" hint="Each gets its own session.">
            <Input
              type="number"
              min={1}
              max={64}
              value={count}
              onChange={(event) =>
                setCount(Math.max(1, Math.min(64, Number(event.target.value) || 1)))
              }
            />
          </Field>
          <Field label="Provider" hint={config ? `default: ${config.openAgentsBin}'s own` : ""}>
            <Input
              value={provider}
              onChange={(event) => setProvider(event.target.value)}
              placeholder="claude"
            />
          </Field>
          <Field label="Model">
            <Input
              value={model}
              onChange={(event) => setModel(event.target.value)}
              placeholder=""
            />
          </Field>
          <Field label="Timeout" hint="seconds">
            <Input
              type="number"
              min={0}
              value={timeout}
              onChange={(event) => setTimeoutSeconds(event.target.value)}
              placeholder="600"
            />
          </Field>
        </div>

        <Field label="Effort" hint="Passed through to the provider.">
          <Input
            value={effort}
            onChange={(event) => setEffort(event.target.value)}
            placeholder="high"
          />
        </Field>

        <CommandLine command={preview} />

        <div className="flex items-center gap-3">
          <Button variant="primary" onClick={launch} disabled={launching || task.trim() === ""}>
            {launching ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <Rocket className="size-3.5" />
            )}
            Launch {count > 1 ? `${count} agents` : "agent"}
          </Button>
          {flash ? <span className="text-success text-xs">{flash}</span> : null}
        </div>

        {error ? (
          <div className="border-destructive/40 bg-destructive/10 text-destructive rounded-md border p-2 text-xs">
            {error}
          </div>
        ) : null}

        {launched ? (
          <ul className="space-y-1 text-xs">
            {launched.map((agent) => (
              <li key={agent.run} className="text-muted-foreground font-mono">
                run {agent.run} · session {agent.session} · pid {agent.pid}
              </li>
            ))}
          </ul>
        ) : null}
      </Card>

      <Card className="text-muted-foreground space-y-3 p-3 text-xs leading-relaxed">
        <h3 className="text-foreground text-sm font-semibold">How a run works</h3>
        <p>
          The service does not call a model. It runs{" "}
          <code className="font-mono">{config?.openAgentsBin ?? "open-agents"} run</code> with the{" "}
          <code className="font-mono">{config?.promptware ?? "Browser"}</code> promptware, and that
          program tells the agent to drive this browser by running{" "}
          <code className="font-mono">ob</code> in a shell.
        </p>
        <p>
          So the agent's entire tool surface is the same command list on the Actions tab. An action
          without a command is an action no agent can perform — which is why they are generated from
          one registry rather than written twice.
        </p>
        <p>
          The promptware's Reflection step writes back what the run learned: site-specific
          selectors, the shape of a flow, corrections to earlier notes. Never credentials, never
          page content.
        </p>
        <p>
          A run that hits a login wall stops and says so. Log in by hand once with{" "}
          <code className="font-mono">ob session start --headed</code>, and the profile keeps the
          session for every run after it.
        </p>
      </Card>
    </div>
  );
}

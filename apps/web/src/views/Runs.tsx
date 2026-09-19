import { Ban, FileText, Loader2, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { api, ApiError } from "@/api/client";
import type { RunRecord } from "@/api/types";
import { CommandLine } from "@/components/CommandLine";
import { Badge, Button, Card, Empty } from "@/components/primitives";
import { StatusDot } from "@/components/StatusDot";
import { cn } from "@/lib/cn";

/** What ran, live.
 *
 * The rows come off the `/api/events` socket rather than a poll, which is what makes a fleet of
 * twenty agents cost one connection. Selecting a run fetches its agent log, which is the only
 * thing here that is not already in the pushed record. */
export function Runs({ runs, connected }: { runs: RunRecord[]; connected: boolean }) {
  const [selected, setSelected] = useState<string | null>(null);
  const active = runs.find((run) => run.id === selected) ?? null;

  return (
    <div className="grid h-full grid-cols-[minmax(280px,380px)_1fr] gap-4 overflow-hidden">
      <Card className="flex min-h-0 flex-col overflow-hidden">
        <header className="flex items-center justify-between border-b px-3 py-2">
          <h2 className="text-sm font-semibold">Runs</h2>
          <span className="text-muted-foreground flex items-center gap-1.5 text-[11px]">
            <span
              className={cn("size-1.5 rounded-full", connected ? "bg-success" : "bg-destructive")}
              aria-hidden
            />
            {connected ? "live" : "reconnecting"}
          </span>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {runs.length === 0 ? <Empty>Nothing has run yet.</Empty> : null}
          {runs.map((run) => (
            <button
              key={run.id}
              type="button"
              onClick={() => setSelected(run.id)}
              className={cn(
                "flex w-full flex-col gap-1 border-b px-3 py-2 text-left transition-colors",
                run.id === selected ? "bg-accent text-accent-foreground" : "hover:bg-accent/40",
              )}
            >
              <span className="flex items-center gap-2">
                <StatusDot status={run.status} />
                <span className="text-muted-foreground font-mono text-[11px]">{run.id}</span>
                <Badge tone={run.kind === "agent" ? "info" : "muted"}>{run.kind}</Badge>
                <span className="text-muted-foreground ml-auto font-mono text-[11px]">
                  {run.session}
                </span>
              </span>
              <span className="line-clamp-2 text-xs">{run.subject}</span>
            </button>
          ))}
        </div>
      </Card>

      {active ? (
        <RunDetail run={active} />
      ) : (
        <Card className="flex items-center justify-center">
          <Empty>Pick a run.</Empty>
        </Card>
      )}
    </div>
  );
}

function RunDetail({ run }: { run: RunRecord }) {
  const [log, setLog] = useState<string>("");
  const [loadingLog, setLoadingLog] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadLog = () => {
    setLoadingLog(true);
    api
      .runLog(run.id)
      .then(setLog)
      .catch((cause: unknown) =>
        setError(cause instanceof ApiError ? cause.message : String(cause)),
      )
      .finally(() => setLoadingLog(false));
  };

  // Refetched whenever the run changes identity or finishes, so a log opened mid-run ends up
  // showing the complete output rather than the first few lines.
  useEffect(() => {
    if (run.kind !== "agent") return;
    loadLog();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run.id, run.status]);

  const cancel = () => {
    api
      .cancelRun(run.id)
      .catch((cause: unknown) =>
        setError(cause instanceof ApiError ? cause.message : String(cause)),
      );
  };

  return (
    <Card className="flex min-h-0 flex-col overflow-hidden">
      <header className="flex items-start justify-between gap-3 border-b p-3">
        <div className="min-w-0">
          <h2 className="flex items-center gap-2 text-sm font-semibold">
            <StatusDot status={run.status} />
            <span className="font-mono">{run.id}</span>
            <Badge>{run.kind}</Badge>
            <Badge
              tone={
                run.status === "succeeded"
                  ? "success"
                  : run.status === "failed"
                    ? "destructive"
                    : run.status === "running"
                      ? "warning"
                      : "muted"
              }
            >
              {run.status}
            </Badge>
          </h2>
          <p className="mt-1 text-xs break-words">{run.subject}</p>
        </div>
        {run.status === "running" ? (
          <Button variant="destructive" size="sm" onClick={cancel}>
            <Ban className="size-3.5" /> Cancel
          </Button>
        ) : null}
      </header>

      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-3">
        <dl className="text-muted-foreground grid grid-cols-2 gap-x-4 gap-y-1 text-[11px]">
          <dt>session</dt>
          <dd className="text-foreground font-mono">{run.session}</dd>
          <dt>started</dt>
          <dd className="text-foreground font-mono">{run.createdAt}</dd>
          {run.finishedAt ? (
            <>
              <dt>finished</dt>
              <dd className="text-foreground font-mono">{run.finishedAt}</dd>
            </>
          ) : null}
          {run.pid ? (
            <>
              <dt>pid</dt>
              <dd className="text-foreground font-mono">{run.pid}</dd>
            </>
          ) : null}
        </dl>

        <CommandLine command={`ob runs show ${run.id}`} />

        {run.error ? (
          <div className="border-destructive/40 bg-destructive/10 text-destructive rounded-md border p-2 text-xs">
            {run.error}
          </div>
        ) : null}

        {run.result ? (
          <section>
            <h3 className="mb-1 text-xs font-semibold">Result</h3>
            <pre className="bg-muted/60 max-h-64 overflow-auto rounded-md border p-2 font-mono text-[11px]">
              {pretty(run.result)}
            </pre>
          </section>
        ) : null}

        {run.kind === "agent" ? (
          <section>
            <div className="mb-1 flex items-center justify-between">
              <h3 className="flex items-center gap-1.5 text-xs font-semibold">
                <FileText className="size-3.5" aria-hidden /> Agent log
              </h3>
              <Button size="sm" variant="ghost" onClick={loadLog} disabled={loadingLog}>
                {loadingLog ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : (
                  <RefreshCw className="size-3.5" />
                )}
              </Button>
            </div>
            <pre className="bg-muted/60 max-h-96 overflow-auto rounded-md border p-2 font-mono text-[11px] whitespace-pre-wrap">
              {log || "empty"}
            </pre>
          </section>
        ) : null}

        {error ? <p className="text-destructive text-xs">{error}</p> : null}
      </div>
    </Card>
  );
}

/** The `result` column holds JSON as a string. Pretty-print it where it parses, and show it raw
 * where it does not — a truncated or non-JSON result is still worth reading. */
function pretty(raw: string): string {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

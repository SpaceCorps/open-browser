import { Eye, EyeOff, Loader2, Plus, Square } from "lucide-react";
import { useState } from "react";
import { api, ApiError } from "@/api/client";
import type { SessionRecord } from "@/api/types";
import { CommandLine } from "@/components/CommandLine";
import { Badge, Button, Card, Empty, Field, Input } from "@/components/primitives";

/** Browser sessions: a named Chrome profile, a running process, and the tab it drives.
 *
 * Stopping one leaves its profile on disk, which is what makes a login survive. Deleting a profile
 * is the thing that logs it out of everything, so it is deliberately not offered here — it belongs
 * behind `ob session remove`, which asks first. */
export function Sessions({
  sessions,
  reload,
  current,
  onSelect,
}: {
  sessions: SessionRecord[];
  reload: () => void;
  current: string;
  onSelect: (name: string) => void;
}) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const act = (label: string, work: Promise<unknown>) => {
    setBusy(label);
    setError(null);
    work
      .then(reload)
      .catch((cause: unknown) =>
        setError(cause instanceof ApiError ? cause.message : String(cause)),
      )
      .finally(() => setBusy(null));
  };

  return (
    <div className="grid h-full grid-cols-1 gap-4 overflow-y-auto lg:grid-cols-[1fr_minmax(260px,340px)]">
      <Card className="flex min-h-0 flex-col overflow-hidden">
        <header className="flex items-center justify-between border-b px-3 py-2">
          <h2 className="text-sm font-semibold">Sessions</h2>
          <span className="text-muted-foreground text-[11px]">{sessions.length} running</span>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {sessions.length === 0 ? <Empty>No session is running. Start one below.</Empty> : null}
          {sessions.map((session) => (
            <div key={session.name} className="flex items-start gap-3 border-b px-3 py-2">
              <button
                type="button"
                onClick={() => onSelect(session.name)}
                className="min-w-0 flex-1 text-left"
              >
                <span className="flex items-center gap-2">
                  <span className="font-mono text-xs font-medium">{session.name}</span>
                  {session.name === current ? <Badge tone="info">active</Badge> : null}
                  <Badge>
                    {session.headless ? (
                      <>
                        <EyeOff className="size-3" aria-hidden /> headless
                      </>
                    ) : (
                      <>
                        <Eye className="size-3" aria-hidden /> visible
                      </>
                    )}
                  </Badge>
                </span>
                <span className="text-muted-foreground mt-0.5 block font-mono text-[11px]">
                  pid {session.pid} · {session.endpoint} · since {session.startedAt}
                </span>
                <span className="text-muted-foreground block truncate font-mono text-[11px]">
                  {session.profile}
                </span>
              </button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => act(session.name, api.stopSession(session.name))}
                disabled={busy === session.name}
                aria-label={`Stop ${session.name}`}
              >
                {busy === session.name ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : (
                  <Square className="size-3.5" />
                )}
              </Button>
            </div>
          ))}
        </div>

        <div className="space-y-2 border-t p-3">
          <Field
            label="Start a session"
            hint="A name with no path separators. Blank uses the configured default."
          >
            <div className="flex gap-2">
              <Input
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="work"
                className="font-mono text-xs"
              />
              <Button
                variant="primary"
                onClick={() =>
                  act(
                    "start",
                    api.startSession(name || undefined).then(() => setName("")),
                  )
                }
                disabled={busy === "start"}
              >
                {busy === "start" ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : (
                  <Plus className="size-3.5" />
                )}
                Start
              </Button>
            </div>
          </Field>
          <CommandLine command={`ob session start ${name || ""}`.trim()} />
          {error ? <p className="text-destructive text-xs">{error}</p> : null}
        </div>
      </Card>

      <Card className="text-muted-foreground space-y-3 p-3 text-xs leading-relaxed">
        <h3 className="text-foreground text-sm font-semibold">Logging in</h3>
        <p>
          A session is a real Chrome profile on disk, so whatever it is logged into stays logged in
          between commands and between runs. That also makes it the most sensitive thing
          open-browser keeps — profiles are excluded from version control and never leave the
          machine.
        </p>
        <p>
          Agents are told to stop at a login wall rather than try credentials. To get past one, run{" "}
          <code className="text-foreground font-mono">ob session start --headed</code>, sign in
          yourself in the window that opens, and every later run in that session inherits it.
        </p>
        <p>
          Stopping a session closes the browser and keeps the profile.{" "}
          <code className="text-foreground font-mono">ob session remove</code> deletes it, which is
          what logs it out of everything.
        </p>
      </Card>
    </div>
  );
}

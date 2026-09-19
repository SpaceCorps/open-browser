import { Bot, Command, History, Layers, MonitorPlay } from "lucide-react";
import { useEffect, useState } from "react";
import { useConfig, useRuns, useSessions } from "@/hooks";
import { Badge } from "@/components/primitives";
import { cn } from "@/lib/cn";
import { Actions } from "@/views/Actions";
import { Agents } from "@/views/Agents";
import { Automations } from "@/views/Automations";
import { Runs } from "@/views/Runs";
import { Sessions } from "@/views/Sessions";

const TABS = [
  { id: "actions", label: "Actions", icon: Command },
  { id: "agents", label: "Agents", icon: Bot },
  { id: "runs", label: "Runs", icon: History },
  { id: "automations", label: "Automations", icon: Layers },
  { id: "sessions", label: "Sessions", icon: MonitorPlay },
] as const;

type Tab = (typeof TABS)[number]["id"];

export default function App() {
  const { data: config } = useConfig();
  const { data: sessions, reload: reloadSessions } = useSessions();
  const { runs, connected } = useRuns();
  const [tab, setTab] = useState<Tab>("actions");
  const [session, setSession] = useState("");

  // The session the whole UI acts in. It comes from the service's own config so that the UI and a
  // bare `ob goto` land in the same browser by default.
  useEffect(() => {
    if (!session && config) setSession(config.session);
  }, [config, session]);

  const running = runs.filter((run) => run.status === "running").length;

  return (
    <div className="dark flex h-screen flex-col overflow-hidden">
      <header className="flex shrink-0 items-center gap-4 border-b px-4 py-2">
        <h1 className="text-sm font-semibold tracking-tight">
          open<span className="text-muted-foreground">-browser</span>
        </h1>

        <nav className="flex items-center gap-1">
          {TABS.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              type="button"
              onClick={() => setTab(id)}
              className={cn(
                "inline-flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-xs font-medium transition-colors",
                tab === id
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent/50",
              )}
            >
              <Icon className="size-3.5" aria-hidden />
              {label}
              {id === "runs" && running > 0 ? <Badge tone="warning">{running}</Badge> : null}
            </button>
          ))}
        </nav>

        <div className="ml-auto flex items-center gap-3 text-[11px]">
          <label className="text-muted-foreground flex items-center gap-1.5">
            session
            <select
              value={session}
              onChange={(event) => setSession(event.target.value)}
              className="border-input bg-background text-foreground h-7 rounded-md border px-1.5 font-mono"
            >
              {/* The configured session is offered even when nothing is running: acting in it is
                  what starts it. */}
              {[...new Set([session, ...(sessions ?? []).map((one) => one.name)])]
                .filter(Boolean)
                .map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
            </select>
          </label>
          <span className="text-muted-foreground flex items-center gap-1.5">
            <span
              className={cn("size-1.5 rounded-full", connected ? "bg-success" : "bg-destructive")}
            />
            {connected ? "connected" : "offline"}
          </span>
        </div>
      </header>

      <main className="min-h-0 flex-1 overflow-hidden p-4">
        {tab === "actions" ? <Actions session={session} /> : null}
        {tab === "agents" ? <Agents session={session} onLaunched={() => setTab("runs")} /> : null}
        {tab === "runs" ? <Runs runs={runs} connected={connected} /> : null}
        {tab === "automations" ? <Automations session={session} /> : null}
        {tab === "sessions" ? (
          <Sessions
            sessions={sessions ?? []}
            reload={reloadSessions}
            current={session}
            onSelect={setSession}
          />
        ) : null}
      </main>
    </div>
  );
}

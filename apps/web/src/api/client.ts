import type {
  ActionOutcome,
  ActionSpec,
  AgentRequest,
  Automation,
  Config,
  EventFrame,
  Health,
  LaunchedAgent,
  RunRecord,
  SessionRecord,
} from "./types";

/** Where the API lives.
 *
 * Three cases, in order. The Tauri shell starts the service in-process on a port the OS picked and
 * injects the address before the first script runs, so that wins. Otherwise the UI is either being
 * served by `ob serve --ui` (same origin) or proxied by the dev server, and in both of those a
 * relative path is correct — which is why the default is the empty string rather than a URL. */
declare global {
  interface Window {
    __OPEN_BROWSER_API__?: string;
  }
}

export const base = (): string => window.__OPEN_BROWSER_API__?.replace(/\/$/, "") ?? "";

/** The error the UI shows. The server answers a failure with `{"error": "..."}` and a status that
 * already distinguishes a bad selector (422) from a dead browser (502), so the message is worth
 * surfacing verbatim rather than replacing with "request failed". */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  // Built through `Headers` rather than by spreading `init.headers`: that field is a union, and
  // spreading the `Headers` or `[name, value][]` arms of it silently produces an object of array
  // indices instead of the headers.
  const headers = new Headers(init?.headers);
  if (init?.body && !headers.has("content-type")) headers.set("content-type", "application/json");

  let response: Response;
  try {
    response = await fetch(`${base()}/api${path}`, { ...init, headers });
  } catch (cause) {
    // A refused connection is the single commonest failure here — the service is simply not
    // running — and "Failed to fetch" does not say that to anyone.
    throw new ApiError(
      0,
      `cannot reach the open-browser service; is \`ob serve\` running? (${String(cause)})`,
    );
  }
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new ApiError(response.status, body?.error ?? `${response.status} ${response.statusText}`);
  }
  if (response.status === 204) return undefined as T;
  const text = await response.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

const post = <T>(path: string, body: unknown): Promise<T> =>
  request<T>(path, { method: "POST", body: JSON.stringify(body) });

export const api = {
  health: () => request<Health>("/health"),
  config: () => request<Config>("/config"),

  actions: () => request<ActionSpec[]>("/actions"),
  /** Run one action. `params` is keyed by the same names the CLI's flags use — the server hands
   * this map to the identical parser `ob click` calls. */
  runAction: (id: string, params: Record<string, string>, session?: string) =>
    post<ActionOutcome>(`/actions/${encodeURIComponent(id)}`, { session, params }),

  sessions: () => request<SessionRecord[]>("/sessions"),
  startSession: (name?: string) => post<SessionRecord>("/sessions", { name }),
  stopSession: (name: string) =>
    request<{ stopped: string }>(`/sessions/${encodeURIComponent(name)}`, { method: "DELETE" }),

  automations: () => request<string[]>("/automations"),
  automation: (name: string) => request<Automation>(`/automations/${encodeURIComponent(name)}`),
  runAutomation: (name: string, inputs: Record<string, string>, session?: string) =>
    post<{ automation: string; ok: boolean; steps: unknown[] }>(
      `/automations/${encodeURIComponent(name)}/run`,
      { session, inputs },
    ),

  /** Launch agents. Returns as soon as they are spawned; progress arrives on the event socket. */
  runAgents: (request_: AgentRequest) => post<LaunchedAgent[]>("/agents", request_),

  runs: (limit = 100) => request<RunRecord[]>(`/runs?limit=${limit}`),
  run: (id: string) => request<RunRecord>(`/runs/${encodeURIComponent(id)}`),
  cancelRun: (id: string) =>
    post<{ cancelled: string }>(`/runs/${encodeURIComponent(id)}/cancel`, {}),
  runLog: (id: string) => request<string>(`/runs/${encodeURIComponent(id)}/log`),
};

/** Subscribe to run updates, reconnecting for as long as the returned function is uncalled.
 *
 * The socket is the reason a fleet of twenty agents does not mean twenty polls a second. It drops
 * whenever `ob serve` restarts, which during development is often, so reconnecting is the normal
 * path rather than error handling. */
export function watchRuns(
  onFrame: (frame: EventFrame) => void,
  onStatus?: (connected: boolean) => void,
): () => void {
  let socket: WebSocket | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let closed = false;
  // Backs off to 5s so a service that is down for a while is not hammered, and resets on connect
  // so the next drop reconnects immediately.
  let delay = 500;

  const connect = () => {
    if (closed) return;
    const origin = base() || window.location.origin;
    const url = new URL("/api/events", origin);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    socket = new WebSocket(url);

    socket.onopen = () => {
      delay = 500;
      onStatus?.(true);
    };
    socket.onmessage = (event) => {
      try {
        onFrame(JSON.parse(String(event.data)) as EventFrame);
      } catch {
        // A frame this build cannot parse is a version skew between the UI and the service, not a
        // reason to tear down a socket that is otherwise delivering.
      }
    };
    socket.onclose = () => {
      onStatus?.(false);
      if (closed) return;
      timer = setTimeout(connect, delay);
      delay = Math.min(delay * 2, 5000);
    };
    socket.onerror = () => socket?.close();
  };

  connect();
  return () => {
    closed = true;
    if (timer) clearTimeout(timer);
    socket?.close();
  };
}

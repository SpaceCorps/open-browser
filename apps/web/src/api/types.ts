/** The shapes `open-browser-server` serialises. Every Rust struct here carries
 * `#[serde(rename_all = "camelCase")]`, so the field names below are the wire names. */

export type ParamKind =
  | "text"
  | "selector"
  | "url"
  | "path"
  | "number"
  | "flag"
  | { choice: string[] };

export interface ParamSpec {
  name: string;
  kind: ParamKind;
  required: boolean;
  repeatable: boolean;
  help: string;
}

/** One browser capability. The palette, the form and the agent's tool list all read this same
 * record — it is served straight from the Rust registry, so the UI cannot drift from the CLI. */
export interface ActionSpec {
  id: string;
  summary: string;
  group: string;
  params: ParamSpec[];
  mutates: boolean;
  example: string;
}

export interface ActionOutcome {
  action: string;
  value: unknown;
  summary: string;
  artifact: string | null;
  durationMs: number;
}

export interface SessionRecord {
  name: string;
  endpoint: string;
  pid: number;
  target?: string;
  profile: string;
  headless: boolean;
  startedAt: string;
}

export type RunKind = "action" | "automation" | "agent";
export type RunStatus = "running" | "succeeded" | "failed" | "cancelled";

export interface RunRecord {
  id: string;
  kind: RunKind;
  status: RunStatus;
  session: string;
  /** The action id, the automation name, or the agent's task. */
  subject: string;
  createdAt: string;
  finishedAt: string | null;
  result: string | null;
  error: string | null;
  pid: number | null;
}

/** A saved action script. `Step.params` is flattened into the step object on the wire, which is
 * why the parameters are `Record<string, string>` alongside the two known keys rather than nested. */
export interface Automation {
  name: string;
  description?: string;
  inputs?: Record<string, string>;
  steps: AutomationStep[];
}

export interface AutomationStep {
  action: string;
  optional?: boolean;
  [param: string]: unknown;
}

export interface LaunchedAgent {
  run: string;
  session: string;
  pid: number;
}

export interface Config {
  session: string;
  headless: boolean;
  window: [number, number];
  chromeArgs: string[];
  openAgentsBin: string;
  promptware: string;
  bind: string;
}

export interface Health {
  ok: boolean;
  actions: number;
  sessions: number;
  version: string;
}

export interface AgentRequest {
  task: string;
  session?: string;
  promptware?: string;
  provider?: string;
  model?: string;
  effort?: string;
  timeoutSeconds?: number;
  count?: number;
}

/** A frame on `/api/events`. `snapshot` carries everything known at connect; `update` carries only
 * the runs whose status changed since the last frame. */
export interface EventFrame {
  type: "snapshot" | "update";
  runs: RunRecord[];
}

/** True for the `Choice` variant, which serde writes as `{"choice": [...]}` while the other
 * variants are bare strings. */
export function choices(kind: ParamKind): string[] | null {
  return typeof kind === "object" && "choice" in kind ? kind.choice : null;
}

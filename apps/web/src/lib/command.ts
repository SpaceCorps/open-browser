import type { ActionSpec, ParamSpec } from "@/api/types";

/** POSIX-quote one argument, the way a person would type it. */
function quote(value: string): string {
  if (value.length > 0 && /^[\w@%+=:,./-]+$/.test(value)) return value;
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

/** Which of an action's required parameters `ob` accepts positionally.
 *
 * Mirrors `positional_count` in `crates/open-browser-cli/src/actions.rs`: the required parameters
 * in declaration order, stopping at the first repeatable one. `tests/command.test.ts` checks this
 * against the live registry, so the two cannot drift silently. */
export function positionalCount(spec: ActionSpec): number {
  const required = spec.params.filter((param) => param.required);
  const repeatable = required.findIndex((param) => param.repeatable);
  return repeatable === -1 ? required.length : repeatable;
}

/** The `ob` invocation equivalent to a set of parameters.
 *
 * Shown beside every action the UI runs, and it is not decoration. The rule this project is built
 * on is that anything doable in the browser has a shell command, so the UI proves it on each
 * action rather than asserting it in a README: what you see is what an agent would run. */
export function cliCommand(
  spec: ActionSpec,
  params: Record<string, string>,
  session?: string,
): string {
  const parts = ["ob"];
  if (session) parts.push("--session", quote(session));
  parts.push(spec.id);

  const positionals = positionalCount(spec);
  let used = 0;
  for (const param of spec.params) {
    const value = params[param.name];
    if (value === undefined || value === "") continue;

    if (param.kind === "flag") {
      // Only a present flag is written. An absent one has to stay absent, or the core's own
      // default (`check` ticks unless told otherwise) is overridden by the preview.
      if (value === "true") parts.push(`--${param.name}`);
      continue;
    }
    if (param.repeatable) {
      for (const one of value.split("\n").filter(Boolean)) {
        parts.push(`--${param.name}=${quote(one)}`);
      }
      continue;
    }
    if (param.required && used < positionals) {
      parts.push(quote(value));
      used += 1;
      continue;
    }
    parts.push(`--${param.name}=${quote(value)}`);
  }
  return parts.join(" ");
}

/** A blank parameter map for an action's form. */
export function emptyParams(spec: ActionSpec): Record<string, string> {
  return Object.fromEntries(spec.params.map((param) => [param.name, ""]));
}

export function isBoolean(param: ParamSpec): boolean {
  return param.kind === "flag";
}

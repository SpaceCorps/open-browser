import { useCallback, useEffect, useRef, useState } from "react";
import { api, ApiError, watchRuns } from "@/api/client";
import type { ActionSpec, Config, RunRecord, SessionRecord } from "@/api/types";

/** One fetch with loading and error state, plus a `reload`. */
export function useResource<T>(load: () => Promise<T>, deps: unknown[] = []) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const reload = useCallback(() => {
    setLoading(true);
    load()
      .then((value) => {
        setData(value);
        setError(null);
      })
      .catch((cause: unknown) =>
        setError(cause instanceof ApiError ? cause.message : String(cause)),
      )
      .finally(() => setLoading(false));
    // `load` is a fresh closure each render; the caller's deps are the real dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  useEffect(reload, [reload]);
  return { data, error, loading, reload, setData };
}

/** Runs, kept current by the event socket rather than by polling.
 *
 * A `snapshot` frame replaces what is known; an `update` frame carries only changed rows and is
 * merged in, newest first. Twenty agents therefore cost one socket, not twenty polls. */
export function useRuns() {
  const [runs, setRuns] = useState<RunRecord[]>([]);
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    return watchRuns((frame) => {
      setRuns((current) => {
        const byId = new Map(frame.type === "snapshot" ? [] : current.map((run) => [run.id, run]));
        for (const run of frame.runs) byId.set(run.id, run);
        return [...byId.values()].sort((a, b) => b.id.localeCompare(a.id));
      });
    }, setConnected);
  }, []);

  return { runs, connected };
}

export function useActions() {
  return useResource<ActionSpec[]>(() => api.actions(), []);
}

export function useSessions() {
  return useResource<SessionRecord[]>(() => api.sessions(), []);
}

export function useConfig() {
  return useResource<Config>(() => api.config(), []);
}

/** A value that reverts to null a few seconds after it is set. For "copied" and "started" notices
 * that should not need dismissing. */
export function useFlash(ms = 2000) {
  const [message, setMessage] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const flash = useCallback(
    (text: string) => {
      setMessage(text);
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => setMessage(null), ms);
    },
    [ms],
  );

  useEffect(() => () => void (timer.current && clearTimeout(timer.current)), []);
  return [message, flash] as const;
}

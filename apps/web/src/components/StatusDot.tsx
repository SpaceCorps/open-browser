import { cn } from "@/lib/cn";
import type { RunStatus } from "@/api/types";

const TONE: Record<RunStatus, string> = {
  running: "bg-warning animate-pulse",
  succeeded: "bg-success",
  failed: "bg-destructive",
  cancelled: "bg-muted-foreground",
};

export function StatusDot({ status, className }: { status: RunStatus; className?: string }) {
  return (
    <span
      role="img"
      aria-label={status}
      className={cn("inline-block size-2 shrink-0 rounded-full", TONE[status], className)}
    />
  );
}

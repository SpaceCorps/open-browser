import { Check, Copy, Terminal } from "lucide-react";
import { useState } from "react";
import { cn } from "@/lib/cn";

/** The `ob` command equivalent to whatever the panel above is about to do.
 *
 * Every surface that performs something shows one of these. It is the project's central claim made
 * visible: the UI has no capability the shell lacks, so an agent — which only has the shell — can
 * do anything a person can do here. */
export function CommandLine({ command, className }: { command: string; className?: string }) {
  const [copied, setCopied] = useState(false);

  const copy = () => {
    void navigator.clipboard
      .writeText(command)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      // Clipboard access is denied in an insecure context, which a bare-IP deployment is. The
      // command is selectable either way, so this is a non-event.
      .catch(() => undefined);
  };

  return (
    <div
      className={cn(
        "bg-muted/60 text-muted-foreground flex items-center gap-2 rounded-md border px-2 py-1.5 font-mono text-xs",
        className,
      )}
    >
      <Terminal className="size-3.5 shrink-0 opacity-60" aria-hidden />
      <code className="text-foreground flex-1 overflow-x-auto whitespace-pre">{command}</code>
      <button
        type="button"
        onClick={copy}
        aria-label="Copy command"
        className="hover:bg-accent hover:text-accent-foreground shrink-0 rounded p-1 transition-colors"
      >
        {copied ? (
          <Check className="size-3.5" aria-hidden />
        ) : (
          <Copy className="size-3.5" aria-hidden />
        )}
      </button>
    </div>
  );
}

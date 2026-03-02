import { Loader2 } from "lucide-react";

export function PageShellFallback() {
  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-8 text-card-foreground">
      <div className="flex items-center gap-3 rounded-lg border border-border bg-card/60 p-4">
        <Loader2 className="h-5 w-5 animate-spin text-emerald-700 dark:text-emerald-300" aria-hidden="true" />
        <span className="text-sm font-medium">Loading...</span>
      </div>
    </div>
  );
}

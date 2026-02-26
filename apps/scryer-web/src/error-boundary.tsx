import { useRouteError, isRouteErrorResponse } from "react-router-dom";
import { AlertTriangle } from "lucide-react";
import { Button } from "@/components/ui/button";

export function RouteErrorBoundary() {
  const error = useRouteError();
  const message = isRouteErrorResponse(error)
    ? `${error.status}: ${error.statusText}`
    : error instanceof Error
      ? error.message
      : "An unexpected error occurred";

  return (
    <div className="mx-auto flex min-h-screen w-full max-w-4xl flex-col items-center justify-center px-6 py-12">
      <div className="w-full max-w-md space-y-4 rounded-lg border border-red-500/30 bg-card p-6">
        <div className="flex items-center gap-3">
          <AlertTriangle className="h-5 w-5 text-red-500" />
          <h1 className="text-lg font-semibold">Something went wrong</h1>
        </div>
        <p className="text-sm text-muted-foreground">An unexpected error occurred while loading this page.</p>
        {message ? <p className="rounded bg-red-500/10 px-3 py-2 text-xs text-red-500">{message}</p> : null}
        <div className="flex justify-end gap-2 pt-2">
          <Button onClick={() => window.location.reload()} type="button" variant="secondary">
            Try again
          </Button>
        </div>
      </div>
    </div>
  );
}

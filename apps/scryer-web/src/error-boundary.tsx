import { useRouteError, isRouteErrorResponse, useNavigate } from "react-router-dom";
import { AlertTriangle, Home, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";

const logoUrl = `${import.meta.env.BASE_URL}logo.webp`;

function errorInfo(error: unknown): { status: number | null; title: string; detail: string } {
  if (isRouteErrorResponse(error)) {
    if (error.status === 404) {
      return {
        status: 404,
        title: "Page not found",
        detail: "The page you're looking for doesn't exist or has been moved.",
      };
    }
    return {
      status: error.status,
      title: error.statusText || "Request failed",
      detail: `The server returned an error (${error.status}).`,
    };
  }

  if (error instanceof Error) {
    return {
      status: null,
      title: "Something went wrong",
      detail: error.message,
    };
  }

  return {
    status: null,
    title: "Unexpected error",
    detail: "An unknown error occurred while loading this page.",
  };
}

export function RouteErrorBoundary() {
  const error = useRouteError();
  const navigate = useNavigate();
  const { status, title, detail } = errorInfo(error);

  return (
    <div className="relative flex min-h-screen items-center justify-center overflow-hidden bg-background px-6 py-12">
      {/* Atmospheric background — matches the app's dark theme glow */}
      <div
        className="pointer-events-none fixed inset-0"
        style={{
          background: [
            "radial-gradient(circle at 30% 20%, rgba(91, 100, 255, 0.08), transparent 40%)",
            "radial-gradient(circle at 70% 80%, rgba(239, 68, 68, 0.06), transparent 40%)",
            "linear-gradient(180deg, transparent 0%, rgba(4, 8, 20, 0.5) 100%)",
          ].join(", "),
        }}
      />

      <div className="relative z-10 flex w-full max-w-md flex-col items-center text-center">
        {/* Logo */}
        <img src={logoUrl} alt="Scryer" className="mb-8 h-16 w-16 opacity-60" />

        {/* Status code */}
        {status !== null && (
          <p className="mb-3 text-6xl font-bold tracking-tight text-foreground/10">{status}</p>
        )}

        {/* Icon + Title */}
        <div className="mb-3 flex items-center justify-center gap-2.5">
          <AlertTriangle className="h-5 w-5 text-red-400" />
          <h1 className="text-lg font-semibold text-foreground">{title}</h1>
        </div>

        {/* Detail */}
        <p className="mb-6 max-w-sm text-sm leading-relaxed text-muted-foreground">{detail}</p>

        {/* Error message (collapsible for long stack traces) */}
        {error instanceof Error && error.stack && (
          <details className="mb-6 w-full text-left">
            <summary className="cursor-pointer text-xs text-muted-foreground/60 hover:text-muted-foreground transition-colors">
              Technical details
            </summary>
            <pre className="mt-2 max-h-48 overflow-auto rounded-md border border-border bg-card/50 p-3 text-[11px] leading-relaxed text-muted-foreground/80">
              {error.stack}
            </pre>
          </details>
        )}

        {/* Actions */}
        <div className="flex items-center gap-3">
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => navigate("/")}
            className="gap-2"
          >
            <Home className="h-3.5 w-3.5" />
            Home
          </Button>
          <Button
            type="button"
            variant="default"
            size="sm"
            onClick={() => window.location.reload()}
            className="gap-2"
          >
            <RefreshCw className="h-3.5 w-3.5" />
            Retry
          </Button>
        </div>
      </div>
    </div>
  );
}

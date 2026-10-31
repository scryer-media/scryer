import { useEffect } from "react";
import { useRouteError, isRouteErrorResponse, useNavigate } from "react-router";
import { AlertTriangle, Home, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { claimViteImportRecovery } from "@/lib/utils/vite-import-recovery";

const logoUrl = `${import.meta.env.BASE_URL}scryer-logo.svg`;
const VITE_IMPORT_RECOVERY_DELAY_MS = 500;

function errorInfo(error: unknown): {
  status: number | null;
  title: string;
  detail: string;
} {
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

  useEffect(() => {
    if (!import.meta.env.PROD || typeof window === "undefined") {
      return;
    }

    if (!claimViteImportRecovery(error, window.sessionStorage)) {
      return;
    }

    const reloadTimer = window.setTimeout(
      () => window.location.reload(),
      VITE_IMPORT_RECOVERY_DELAY_MS,
    );
    return () => window.clearTimeout(reloadTimer);
  }, [error]);

  return (
    <div className="flex min-h-dvh items-center justify-center bg-[#070d1c] px-6 py-12 text-[#dbe5ff] [color-scheme:dark]">
      <div className="flex w-full min-w-0 max-w-md flex-col items-center text-center">
        {/* Logo */}
        <img src={logoUrl} alt="Scryer" className="mb-8 h-16 w-16 opacity-60" />

        {/* Status code */}
        {status !== null && (
          <p className="mb-3 text-6xl font-bold tracking-tight text-[#8b96b9]">
            {status}
          </p>
        )}

        {/* Icon + Title */}
        <div className="mb-3 flex items-center justify-center gap-2.5">
          <AlertTriangle className="h-5 w-5 shrink-0 text-[#f87171]" />
          <h1 className="text-lg font-semibold">{title}</h1>
        </div>

        {/* Detail */}
        <p className="mb-6 max-w-sm text-sm leading-relaxed text-[#8b96b9] [overflow-wrap:anywhere]">
          {detail}
        </p>

        {/* Error message (collapsible for long stack traces) */}
        {error instanceof Error && error.stack && (
          <details className="mb-6 w-full text-left">
            <summary className="cursor-pointer text-xs text-[#8b96b9] transition-colors hover:text-[#dbe5ff] focus-visible:outline-2 focus-visible:outline-[#7b8cff]">
              Technical details
            </summary>
            <pre className="mt-2 max-h-48 overflow-auto rounded-md border border-[#1c2746] bg-[#0a1120] p-3 text-[11px] leading-relaxed text-[#8b96b9]">
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
            className="gap-2 border-[#1c2746] bg-[#0b1322] text-[#dbe5ff] hover:bg-[#151f35] hover:text-[#dbe5ff] focus-visible:ring-[#7b8cff]"
          >
            <Home className="h-3.5 w-3.5" />
            Home
          </Button>
          <Button
            type="button"
            variant="default"
            size="sm"
            onClick={() => window.location.reload()}
            className="gap-2 bg-[#5b64ff] text-white hover:bg-[#4a52e6] focus-visible:ring-[#7b8cff]"
          >
            <RefreshCw className="h-3.5 w-3.5" />
            Retry
          </Button>
        </div>
      </div>
    </div>
  );
}

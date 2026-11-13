import type { Translate } from "@/components/root/types";

/**
 * Why the server refused or failed a diagnostics bundle download. The route
 * answers with `{ error, reason }`; each reason listed here is said in the
 * viewer's language, and any other failure shows the server's sentence.
 */
export const DIAGNOSTICS_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED:
    "system.diagnosticsBundle.refusal.authenticationRequired",
  HTTP_DIAGNOSTICS_PERMISSION_DENIED:
    "system.diagnosticsBundle.refusal.permissionDenied",
  HTTP_DIAGNOSTICS_BUNDLE_FAILED: "system.diagnosticsBundle.refusal.failed",
};

/** A failed diagnostics download: the server's sentence and its reason code. */
export class DiagnosticsBundleError extends Error {
  readonly reason: string | null;

  constructor(message: string, reason: string | null) {
    super(message);
    this.name = "DiagnosticsBundleError";
    this.reason = reason;
  }
}

/** Reads a non-2xx diagnostics response into a `DiagnosticsBundleError`. */
export async function diagnosticsBundleErrorFromResponse(
  response: Response,
): Promise<DiagnosticsBundleError> {
  const contentType = response.headers.get("content-type") ?? "";
  if (contentType.includes("application/json")) {
    const payload = (await response.json().catch(() => null)) as {
      error?: unknown;
      reason?: unknown;
    } | null;
    return new DiagnosticsBundleError(
      typeof payload?.error === "string" ? payload.error.trim() : "",
      typeof payload?.reason === "string" ? payload.reason : null,
    );
  }
  const text = await response.text().catch(() => "");
  return new DiagnosticsBundleError(text.trim(), null);
}

/**
 * What to show when the download fails: a known reason in the viewer's
 * language, otherwise the server's sentence, otherwise `fallback`.
 */
export function diagnosticsBundleErrorMessage(
  error: unknown,
  t: Translate,
  fallback: string,
): string {
  if (error instanceof DiagnosticsBundleError) {
    if (
      error.reason !== null &&
      Object.hasOwn(DIAGNOSTICS_REFUSAL_KEYS, error.reason)
    ) {
      return t(DIAGNOSTICS_REFUSAL_KEYS[error.reason]);
    }
    return error.message || fallback;
  }
  return fallback;
}

// Download the sanitized diagnostics bundle from the server logs page. The
// request carries an Authorization header, so it cannot be a plain link; the
// browser's own download is the confirmation.
import { getAuthToken } from "@/lib/hooks/use-auth";
import { scryerFetch } from "@/lib/graphql/urql-client";
import { buildAppUrl } from "@/lib/runtime-config";
import { diagnosticsBundleErrorFromResponse } from "@/lib/utils/diagnostics-bundle-error";
import {
  filenameFromContentDisposition,
  saveDownloadResponse,
} from "@/lib/utils/save-download-response";

const DIAGNOSTICS_BUNDLE_PATH = "/api/diagnostics/bundle";
export const DIAGNOSTICS_BUNDLE_FALLBACK_FILENAME =
  "scryer-diagnostics.tar.zst";

/** Throws a `DiagnosticsBundleError` when the server refuses or fails. */
export async function downloadDiagnosticsBundle(): Promise<void> {
  const token = getAuthToken();
  const response = await scryerFetch(buildAppUrl(DIAGNOSTICS_BUNDLE_PATH), {
    method: "GET",
    // Authless local mode has no token; `scryerFetch` handles that request.
    headers: token ? { Authorization: `Bearer ${token}` } : {},
  });
  if (!response.ok) {
    throw await diagnosticsBundleErrorFromResponse(response);
  }
  const filename = filenameFromContentDisposition(
    response.headers.get("content-disposition"),
    DIAGNOSTICS_BUNDLE_FALLBACK_FILENAME,
  );
  await saveDownloadResponse(response, filename);
}

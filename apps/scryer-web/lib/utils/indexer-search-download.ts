// Download the selected Indexers > Search releases to the browser (D17,
// FR-028). One release answers with its own `.nzb`/`.torrent`, several with a
// single `tar.gz`; each one is recorded in History as a grab. Nothing is
// queued, so there is no submission to follow and no toast on success — the
// browser's own download is the confirmation.
import { getAuthToken } from "@/lib/hooks/use-auth";
import { scryerFetch } from "@/lib/graphql/urql-client";
import { buildAppUrl } from "@/lib/runtime-config";
import {
  filenameFromContentDisposition,
  readResponseErrorMessage,
  saveDownloadResponse,
} from "@/lib/utils/save-download-response";

const ARTIFACTS_PATH = "/api/indexer-search/artifacts";
export const SINGLE_RELEASE_FALLBACK_FILENAME = "scryer-release.nzb";
export const BUNDLE_FALLBACK_FILENAME = "scryer-releases.tar.gz";

export type IndexerSearchArtifactTarget = {
  searchId: string;
  downloadUrl: string;
};

export async function downloadIndexerSearchArtifacts({
  releases,
  failureMessage,
}: {
  /** Each release names the job it came from: a selection can span jobs. */
  releases: IndexerSearchArtifactTarget[];
  /** Shown when the server answers with no message of its own. */
  failureMessage: string;
}): Promise<void> {
  const token = getAuthToken();
  const response = await scryerFetch(buildAppUrl(ARTIFACTS_PATH), {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      // Authless local mode has no token; `scryerFetch` handles that request.
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    },
    body: JSON.stringify({ releases }),
  });
  if (!response.ok) {
    throw new Error(await readResponseErrorMessage(response, failureMessage));
  }
  const filename = filenameFromContentDisposition(
    response.headers.get("content-disposition"),
    releases.length > 1
      ? BUNDLE_FALLBACK_FILENAME
      : SINGLE_RELEASE_FALLBACK_FILENAME,
  );
  await saveDownloadResponse(response, filename);
}

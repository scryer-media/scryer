import type { Operation } from "@urql/core";

// Requests that wait on something slow outside the database: an indexer
// handing over a release, a download client accepting it, an archive being
// extracted, a folder on a network share being scanned, a metadata refresh.
//
// A browser allows six HTTP/1.1 connections per origin. A handful of these in
// flight would leave every other request of the page waiting for a free
// connection, so they are sent through the slow lane (`slow-lane.ts`), which
// lets only a few of them hold a connection at a time.
export const LONG_OPERATION_NAMES: ReadonlySet<string> = new Set([
  // Adding a title and grabbing its first release in one request. Every
  // other grab runs as a background job (`use-grab.ts`) and holds no request.
  "AddTitleAndQueue",
  // Imports and scans.
  "RetryImport",
  "ReleaseHeldImportSources",
  "BeginManualImportSelection",
  "ResolvePendingImport",
  "BindPendingImport",
  "ScanTitleLibrary",
  "FixTitleMatch",
  "ApplyTitleFolderChange",
  // Subtitle provider search.
  "SearchSubtitles",
  // Connection tests and previews: each waits on the service being tested.
  "TestProxyConfig",
  "TestIndexerConnection",
  "TestDownloadClientConnection",
  "TestSubtitleProviderConnection",
  "TestMediaServerConnection",
  "TestEmbyConnect",
  "TestNotificationChannel",
  "ValidateExternalImportConnection",
  "PreviewExternalImport",
  // Download client commands.
  "PauseDownload",
  "ResumeDownload",
]);

function operationName(operation: Operation): string | undefined {
  for (const definition of operation.query.definitions) {
    if (definition.kind === "OperationDefinition") {
      return definition.name?.value;
    }
  }
  return undefined;
}

export function isLongOperation(operation: Operation): boolean {
  if (operation.kind !== "mutation" && operation.kind !== "query") {
    return false;
  }
  const name = operationName(operation);
  return name !== undefined && LONG_OPERATION_NAMES.has(name);
}

import type { TitleHistoryEvent } from "@/lib/types";
import { LazyCodeEditor } from "@/components/common/lazy-code-editor";
import { formatSanitizedHistoryValue } from "@/lib/utils/history-redaction";
import { cn } from "@/lib/utils";
import { titleMoveHistoryDetails } from "@/lib/utils/history-moves";

const friendlyKeys: Record<string, string> = {
  import_id: "Import ID",
  client_id: "Client ID",
  client_name: "Client Name",
  client_type: "Client Type",
  indexer: "Indexer",
  download_client: "Download Client",
  download_client_name: "Client Name",
  download_id: "Download ID",
  download_url: "Download URL",
  nzb_info_url: "NZB Info",
  release_group: "Release Group",
  size: "Size",
  source_path: "Source Path",
  source_ref: "Source Ref",
  source_hint: "Source Hint",
  dest_path: "Destination",
  dropped_path: "Dropped Path",
  imported_path: "Imported Path",
  reason: "Reason",
  message: "Message",
  age: "Age",
  protocol: "Protocol",
  indexer_flags: "Indexer Flags",
  release_type: "Release Type",
  source_system: "Source System",
  skip_reason: "Skip Reason",
  error_message: "Error",
  blocklist_reason: "Blocklist Reason",
};

const structuredDataKeys = new Set([
  "import_id",
  "download_id",
  "client_id",
  "client_name",
  "source_system",
  "source_ref",
  "source_hint",
  "skip_reason",
  "source_path",
  "dest_path",
  "reason",
  "blocklist_reason",
]);

function formatKey(key: string): string {
  return friendlyKeys[key] ?? key.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

function formatValue(value: unknown, key?: string): string {
  return formatSanitizedHistoryValue(value, key);
}

function formatJsonCode(value: unknown, key?: string): string {
  let parsed = value;
  if (typeof value === "string") {
    try {
      parsed = JSON.parse(value);
    } catch {
      return formatValue(value, key);
    }
  }

  const sanitized = formatSanitizedHistoryValue(parsed, key);
  try {
    return JSON.stringify(JSON.parse(sanitized), null, 2);
  } catch {
    return sanitized;
  }
}

const ignoreReadOnlyCodeChange = (_value: string) => undefined;

function parseDataJson(raw: unknown): Record<string, unknown> | null {
  if (!raw) return null;
  try {
    const parsed = raw;
    if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)) {
      return parsed as Record<string, unknown>;
    }
    return null;
  } catch {
    return null;
  }
}

type HistoryEventDetailEntry = {
  key: string;
  value: string;
};

export function buildHistoryEventDetail(event: TitleHistoryEvent): {
  structuredDetails: HistoryEventDetailEntry[];
  rawDetails: Array<[string, unknown]>;
  hasDetail: boolean;
} {
  const data = parseDataJson(event.dataJson);
  const moveDetails = titleMoveHistoryDetails(event);
  if (moveDetails) {
    return { structuredDetails: moveDetails, rawDetails: [], hasDetail: true };
  }
  const structuredDetails = [
    event.importId ? { key: "import_id", value: event.importId } : null,
    event.downloadId
      ? { key: "download_id", value: event.downloadId }
      : null,
    event.clientId ? { key: "client_id", value: event.clientId } : null,
    event.clientName ? { key: "client_name", value: event.clientName } : null,
    event.sourceSystem ? { key: "source_system", value: event.sourceSystem } : null,
    event.sourceRef ? { key: "source_ref", value: event.sourceRef } : null,
    event.sourceProvider
      ? { key: "source_provider", value: event.sourceProvider }
      : event.sourceHint
        ? { key: "source_hint", value: event.sourceHint }
        : null,
    event.skipReason ? { key: "skip_reason", value: event.skipReason } : null,
    event.sourcePath ? { key: "source_path", value: event.sourcePath } : null,
    event.destPath ? { key: "dest_path", value: event.destPath } : null,
    event.failureReason ? { key: "reason", value: event.failureReason } : null,
    event.blocklistReason
      ? { key: "blocklist_reason", value: event.blocklistReason }
      : null,
  ].filter((entry): entry is HistoryEventDetailEntry => entry !== null);
  const rawDetails = Object.entries(data ?? {}).filter(
    ([key]) => !structuredDataKeys.has(key),
  );
  const hasDetail = structuredDetails.length > 0 || rawDetails.length > 0;

  return {
    structuredDetails,
    rawDetails,
    hasDetail,
  };
}

export function HistoryEventDetailContent({ event }: { event: TitleHistoryEvent }) {
  const { structuredDetails, rawDetails, hasDetail } = buildHistoryEventDetail(event);

  if (!hasDetail) {
    return null;
  }

  return (
    <div className="space-y-1.5">
      {structuredDetails.map(({ key, value }) => (
        <div key={key} className="grid grid-cols-[auto_1fr] gap-x-3 text-xs">
          <span className="whitespace-nowrap text-muted-foreground">{formatKey(key)}</span>
          <span className={cn("break-all text-foreground", key.endsWith("_path") && "font-[var(--font-code)]")}>
            {formatValue(value, key)}
          </span>
        </div>
      ))}
      {rawDetails.map(([key, value]) =>
        key === "data" ? (
          <div key={key} className="space-y-1.5 text-xs">
            <div className="text-muted-foreground">{formatKey(key)}</div>
            <LazyCodeEditor
              id={`history-event-data-${event.id}`}
              value={formatJsonCode(value, key)}
              onChange={ignoreReadOnlyCodeChange}
              readOnly
              language="javascript"
              height="240px"
            />
          </div>
        ) : (
          <div key={key} className="grid grid-cols-[auto_1fr] gap-x-3 text-xs">
            <span className="whitespace-nowrap text-muted-foreground">{formatKey(key)}</span>
            <span className="break-all text-foreground">{formatValue(value, key)}</span>
          </div>
        ),
      )}
    </div>
  );
}

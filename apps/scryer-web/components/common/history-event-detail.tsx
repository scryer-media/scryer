import { Info } from "lucide-react";
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@/components/ui/hover-card";
import type { TitleHistoryEvent } from "@/lib/types";
import { HistoryEventIcon, getEventTypeLabel } from "./history-event-icon";

const friendlyKeys: Record<string, string> = {
  indexer: "Indexer",
  download_client: "Download Client",
  download_client_name: "Client Name",
  download_url: "Download URL",
  nzb_info_url: "NZB Info",
  release_group: "Release Group",
  size: "Size",
  source_path: "Source Path",
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
  error_message: "Error",
};

function formatKey(key: string): string {
  return friendlyKeys[key] ?? key.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

function formatValue(value: unknown): string {
  if (value === null || value === undefined) return "\u2014";
  if (typeof value === "string") return value;
  if (typeof value === "number") return String(value);
  if (typeof value === "boolean") return value ? "Yes" : "No";
  return JSON.stringify(value);
}

function parseDataJson(raw: string | null): Record<string, unknown> | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)) {
      return parsed as Record<string, unknown>;
    }
    return null;
  } catch {
    return null;
  }
}

export function HistoryEventDetailHover({ event }: { event: TitleHistoryEvent }) {
  const data = parseDataJson(event.dataJson);
  const hasDetail = data && Object.keys(data).length > 0;

  if (!hasDetail) return null;

  return (
    <HoverCard openDelay={250} closeDelay={75}>
      <HoverCardTrigger asChild>
        <button
          type="button"
          className="inline-flex items-center justify-center rounded p-1 text-muted-foreground hover:text-foreground"
          aria-label="Event details"
        >
          <Info className="h-4 w-4" />
        </button>
      </HoverCardTrigger>
      <HoverCardContent side="left" sideOffset={8} className="w-80 max-h-96 overflow-y-auto">
        <div className="mb-2 flex items-center gap-2">
          <HistoryEventIcon eventType={event.eventType} size={14} />
          <span className="text-xs font-medium">{getEventTypeLabel(event.eventType)}</span>
        </div>
        <div className="space-y-1.5">
          {Object.entries(data).map(([key, value]) => (
            <div key={key} className="grid grid-cols-[auto_1fr] gap-x-3 text-xs">
              <span className="text-muted-foreground whitespace-nowrap">{formatKey(key)}</span>
              <span className="break-all text-foreground">{formatValue(value)}</span>
            </div>
          ))}
        </div>
      </HoverCardContent>
    </HoverCard>
  );
}

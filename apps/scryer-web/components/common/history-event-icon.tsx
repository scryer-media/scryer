import {
  ArrowDownToLine,
  Ban,
  CheckCircle2,
  FileEdit,
  HardDrive,
  SkipForward,
  Trash2,
  XCircle,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

type EventIconConfig = {
  icon: LucideIcon;
  className: string;
  label: string;
};

const eventTypeConfig: Record<string, EventIconConfig> = {
  grabbed: {
    icon: ArrowDownToLine,
    className: "text-sky-400",
    label: "Grabbed",
  },
  download_completed: {
    icon: CheckCircle2,
    className: "text-emerald-400",
    label: "Downloaded",
  },
  imported: {
    icon: HardDrive,
    className: "text-emerald-400",
    label: "Imported",
  },
  import_failed: {
    icon: XCircle,
    className: "text-rose-400",
    label: "Import Failed",
  },
  import_skipped: {
    icon: SkipForward,
    className: "text-amber-400",
    label: "Import Skipped",
  },
  file_deleted: {
    icon: Trash2,
    className: "text-rose-400",
    label: "Deleted",
  },
  file_renamed: {
    icon: FileEdit,
    className: "text-cyan-400",
    label: "Renamed",
  },
  download_ignored: {
    icon: Ban,
    className: "text-amber-400",
    label: "Ignored",
  },
};

const fallbackConfig: EventIconConfig = {
  icon: HardDrive,
  className: "text-muted-foreground",
  label: "Unknown",
};

export function HistoryEventIcon({
  eventType,
  size = 16,
}: {
  eventType: string;
  size?: number;
}) {
  const config = eventTypeConfig[eventType] ?? fallbackConfig;
  const Icon = config.icon;
  return (
    <Icon
      style={{ width: size, height: size }}
      className={`shrink-0 ${config.className}`}
      aria-label={config.label}
    />
  );
}

export function getEventTypeLabel(eventType: string): string {
  return (eventTypeConfig[eventType] ?? fallbackConfig).label;
}

const eventTypeBadgeClasses: Record<string, string> = {
  grabbed: "border-sky-500/40 bg-sky-500/10 text-sky-200",
  download_completed:
    "border-emerald-500/40 bg-emerald-500/10 text-emerald-200",
  imported: "border-emerald-500/40 bg-emerald-500/10 text-emerald-200",
  import_failed: "border-rose-500/40 bg-rose-500/10 text-rose-200",
  import_skipped: "border-amber-500/40 bg-amber-500/10 text-amber-200",
  file_deleted: "border-rose-500/40 bg-rose-500/10 text-rose-200",
  file_renamed: "border-cyan-500/40 bg-cyan-500/10 text-cyan-200",
  download_ignored: "border-amber-500/40 bg-amber-500/10 text-amber-200",
};

export function getEventTypeBadgeClass(eventType: string): string {
  return (
    eventTypeBadgeClasses[eventType] ??
    "border-border bg-muted text-card-foreground"
  );
}

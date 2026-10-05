import {
  AlertTriangle,
  ArrowDownToLine,
  Ban,
  ArchiveRestore,
  EyeOff,
  FileEdit,
  Gavel,
  HardDrive,
  Replace,
  RefreshCcw,
  Share2,
  SkipForward,
  Trash2,
  Undo2,
  XCircle,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

export const TITLE_HISTORY_FILTERS = [
  "grabbed",
  "download_failed",
  "blocklisted",
  "download_ignored",
  "scanned",
  "imported",
  "import_failed",
  "import_skipped",
  "import_rejected_by_rule",
  "file_upgraded",
  "file_recycled",
  "file_deleted",
  "file_restored",
  "file_renamed",
  "title_moved",
  "rematched",
  "seeding_started",
  "seeding_completed",
] as const;

export const WANTED_HISTORY_FILTERS = [
  "grabbed",
  "download_failed",
  "blocklisted",
  "download_ignored",
  "imported",
  "import_failed",
  "import_skipped",
  "import_rejected_by_rule",
] as const;

type EventMeta = {
  icon: LucideIcon;
  iconClassName: string;
  labelKey: string;
  badgeClassName: string;
};

const eventMeta: Record<string, EventMeta> = {
  grabbed: {
    icon: ArrowDownToLine,
    iconClassName: "text-[var(--scry-info-text-soft)]",
    labelKey: "history.grabbed",
    badgeClassName: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  },
  download_failed: {
    icon: AlertTriangle,
    iconClassName: "text-[var(--scry-danger-text-soft)]",
    labelKey: "history.downloadFailed",
    badgeClassName: "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
  },
  blocklisted: {
    icon: Ban,
    iconClassName: "text-[var(--scry-warning-text)]",
    labelKey: "history.blocklisted",
    badgeClassName: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
  },
  scanned: {
    icon: HardDrive,
    iconClassName: "text-[var(--scry-info-text-soft)]",
    labelKey: "history.scanned",
    badgeClassName: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  },
  imported: {
    icon: HardDrive,
    iconClassName: "text-[var(--scry-success-text-soft)]",
    labelKey: "history.imported",
    badgeClassName: "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]",
  },
  import_failed: {
    icon: XCircle,
    iconClassName: "text-[var(--scry-danger-text-soft)]",
    labelKey: "history.importFailed",
    badgeClassName: "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
  },
  import_skipped: {
    icon: SkipForward,
    iconClassName: "text-[var(--scry-warning-text)]",
    labelKey: "history.importSkipped",
    badgeClassName: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
  },
  // A rule doing what the operator wrote is not a failure: accent styling, not
  // the danger or warning palettes failures and skips use.
  import_rejected_by_rule: {
    icon: Gavel,
    iconClassName: "text-[var(--scry-accent-text)]",
    labelKey: "history.importRejectedByRule",
    badgeClassName: "border-[rgba(var(--scry-accent-rgb),0.4)] bg-[rgba(var(--scry-accent-rgb),0.1)] text-[var(--scry-accent-text)]",
  },
  file_upgraded: {
    icon: Replace,
    iconClassName: "text-[var(--scry-success-text-soft)]",
    labelKey: "history.fileUpgraded",
    badgeClassName: "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]",
  },
  file_recycled: {
    icon: ArchiveRestore,
    iconClassName: "text-[var(--scry-warning-text)]",
    labelKey: "history.fileRecycled",
    badgeClassName: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
  },
  file_deleted: {
    icon: Trash2,
    iconClassName: "text-[var(--scry-danger-text-soft)]",
    labelKey: "history.fileDeleted",
    badgeClassName: "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
  },
  file_restored: {
    icon: Undo2,
    iconClassName: "text-[var(--scry-success-text-soft)]",
    labelKey: "history.fileRestored",
    badgeClassName: "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]",
  },
  file_renamed: {
    icon: FileEdit,
    iconClassName: "text-[var(--scry-info-text-soft)]",
    labelKey: "history.fileRenamed",
    badgeClassName: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  },
  title_moved: {
    icon: HardDrive,
    iconClassName: "text-[var(--scry-info-text-soft)]",
    labelKey: "history.titleMoved",
    badgeClassName: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  },
  seeding_started: {
    icon: Share2,
    iconClassName: "text-[var(--scry-info-text-soft)]",
    labelKey: "history.seedingStarted",
    badgeClassName: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  },
  seeding_completed: {
    icon: Share2,
    iconClassName: "text-[var(--scry-success-text-soft)]",
    labelKey: "history.seedingCompleted",
    badgeClassName: "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]",
  },
  download_ignored: {
    icon: EyeOff,
    iconClassName: "text-[var(--scry-warning-text)]",
    labelKey: "history.downloadIgnored",
    badgeClassName: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
  },
  rematched: {
    icon: RefreshCcw,
    iconClassName: "text-[var(--scry-accent-text)]",
    labelKey: "history.rematched",
    badgeClassName: "border-[rgba(var(--scry-accent-rgb),0.4)] bg-[rgba(var(--scry-accent-rgb),0.1)] text-[var(--scry-accent-text)]",
  },
};

const fallbackMeta: EventMeta = {
  icon: HardDrive,
  iconClassName: "text-muted-foreground",
  labelKey: "history.unknownEvent",
  badgeClassName: "border-border bg-muted text-card-foreground",
};

export function getTitleHistoryEventMeta(eventType: string): EventMeta {
  return eventMeta[eventType] ?? fallbackMeta;
}

export function getTitleHistoryEventLabel(
  eventType: string,
  translate: (key: string) => string,
): string {
  return translate(getTitleHistoryEventMeta(eventType).labelKey);
}

export function getTitleHistoryFilterLabel(
  eventType: string,
  translate: (key: string) => string,
): string {
  return getTitleHistoryEventLabel(eventType, translate);
}

// History rows are read straight from the domain event log, so each history
// event type is produced by exactly these domain events. A history view
// refetches when one of them arrives.
const DOMAIN_EVENT_TYPES_BY_HISTORY_EVENT: Record<string, readonly string[]> = {
  grabbed: ["RELEASE_GRABBED"],
  download_failed: ["DOWNLOAD_FAILED"],
  blocklisted: ["RELEASE_BLOCKLISTED"],
  download_ignored: ["DOWNLOAD_IGNORED"],
  scanned: ["MEDIA_FILE_ANALYZED"],
  imported: ["IMPORT_COMPLETED"],
  import_failed: ["IMPORT_REJECTED"],
  import_skipped: ["IMPORT_REJECTED"],
  import_rejected_by_rule: ["IMPORT_REJECTED"],
  file_upgraded: ["MEDIA_FILE_UPGRADED"],
  file_recycled: ["MEDIA_FILE_DELETED"],
  file_deleted: ["MEDIA_FILE_DELETED"],
  file_renamed: ["MEDIA_FILE_RENAMED"],
  file_restored: ["MEDIA_FILE_RESTORED"],
  title_moved: ["TITLE_MOVED"],
  rematched: ["TITLE_REMATCHED"],
  seeding_started: ["SEEDING_STARTED"],
  seeding_completed: ["SEEDING_COMPLETED"],
};

/** The domain event types that can add a row to a history view showing `historyEventTypes`. */
export function domainEventTypesForHistoryEvents(
  historyEventTypes: readonly string[],
): string[] {
  const types = new Set<string>();
  for (const historyEventType of historyEventTypes) {
    for (const domainEventType of DOMAIN_EVENT_TYPES_BY_HISTORY_EVENT[historyEventType] ?? []) {
      types.add(domainEventType);
    }
  }
  return [...types].sort();
}

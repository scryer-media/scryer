import { Loader2, RotateCcw, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
} from "@/lib/utils/action-button-styles";

export type RecycledItem = {
  id: string;
  originalPath: string;
  fileName: string;
  sizeBytes: number;
  titleId: string | null;
  reason: string;
  recycledAt: string;
  mediaRoot: string;
};

type Props = {
  items: RecycledItem[];
  loading: boolean;
  mutatingId: string | null;
  onRestore: (item: RecycledItem) => void;
  onDelete: (item: RecycledItem) => void;
  onEmptyAll: () => void;
};

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return iso;
  }
}

const REASON_LABELS: Record<string, { label: string; className: string }> = {
  upgrade_replaced: { label: "Upgrade", className: "bg-blue-900/40 text-blue-300" },
  file_deleted: { label: "Deleted", className: "bg-red-900/40 text-red-300" },
  invalid_file: { label: "Invalid", className: "bg-yellow-900/40 text-yellow-300" },
  language_mismatch: { label: "Language", className: "bg-orange-900/40 text-orange-300" },
  post_download_rule_blocked: { label: "Rule blocked", className: "bg-purple-900/40 text-purple-300" },
};

function ReasonBadge({ reason }: { reason: string }) {
  const info = REASON_LABELS[reason] ?? { label: reason, className: "bg-muted text-muted-foreground" };
  return (
    <span className={`rounded px-1.5 py-0.5 text-xs ${info.className}`}>
      {info.label}
    </span>
  );
}

export function SettingsRecycleBinSection({
  items,
  loading,
  mutatingId,
  onRestore,
  onDelete,
  onEmptyAll,
}: Props) {
  const t = useTranslate();

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("label.loading")}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <p className="text-sm text-muted-foreground">{t("settings.recycleBinSection")}</p>
        <Button
          variant="outline"
          size="sm"
          disabled={items.length === 0 || mutatingId !== null}
          onClick={onEmptyAll}
          className="text-red-400 hover:text-red-300"
        >
          <Trash2 className="mr-2 h-4 w-4" />
          {t("settings.recycleBinEmptyAll")}
        </Button>
      </div>

      {items.length === 0 ? (
        <p className="py-4 text-sm text-muted-foreground">{t("settings.recycleBinEmpty")}</p>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("label.name")}</TableHead>
              <TableHead>Reason</TableHead>
              <TableHead>Size</TableHead>
              <TableHead>Recycled</TableHead>
              <TableHead className="text-right">{t("label.actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {items.map((item) => {
              const isBusy = mutatingId === item.id || mutatingId === "__empty__";
              return (
                <TableRow key={item.id}>
                  <TableCell>
                    <div>
                      <div className="font-medium">{item.fileName}</div>
                      <div className="max-w-[300px] truncate text-xs text-muted-foreground" title={item.originalPath}>
                        {item.originalPath}
                      </div>
                    </div>
                  </TableCell>
                  <TableCell>
                    <ReasonBadge reason={item.reason} />
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground whitespace-nowrap">
                    {formatSize(item.sizeBytes)}
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground whitespace-nowrap">
                    {formatDate(item.recycledAt)}
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex items-center justify-end gap-1">
                      <Button
                        type="button"
                        size="icon-sm"
                        variant="secondary"
                        title={t("settings.recycleBinRestore")}
                        aria-label={t("settings.recycleBinRestore")}
                        disabled={isBusy}
                        onClick={() => onRestore(item)}
                        className={cn(
                          boxedActionButtonBaseClass,
                          boxedActionButtonToneClass.enabled,
                        )}
                      >
                        <RotateCcw className="h-4 w-4" />
                      </Button>
                      <Button
                        type="button"
                        size="icon-sm"
                        variant="secondary"
                        title={t("settings.recycleBinDelete")}
                        aria-label={t("settings.recycleBinDelete")}
                        disabled={isBusy}
                        onClick={() => onDelete(item)}
                        className={cn(
                          boxedActionButtonBaseClass,
                          boxedActionButtonToneClass.delete,
                        )}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      )}
    </div>
  );
}

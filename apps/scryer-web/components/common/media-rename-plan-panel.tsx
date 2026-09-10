import { ArrowDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";

export type MediaRenamePlanItem = {
  collectionId?: string | null;
  seriesMovieLinkIds?: string[];
  currentPath?: string | null;
  proposedPath?: string | null;
  normalizedFilename?: string | null;
  collision?: boolean;
  reasonCode?: string | null;
  writeAction?: string | null;
  sourceSizeBytes?: number | null;
  sourceMtimeUnixMs?: number | null;
};

export type MediaRenamePlan = {
  facet?: string;
  titleId?: string | null;
  template?: string;
  collisionPolicy?: string;
  missingMetadataPolicy?: string;
  fingerprint: string;
  total: number;
  renamable: number;
  noop: number;
  conflicts: number;
  errors: number;
  items: MediaRenamePlanItem[];
};

export function MediaRenamePlanPanel({
  plan,
  applying,
  applyDisabled,
  onApply,
  onCancel,
  applyButtonId,
}: {
  plan: MediaRenamePlan;
  applying: boolean;
  applyDisabled: boolean;
  onApply: () => void;
  // Omit to render the plan without a way to dismiss it.
  onCancel?: () => void;
  applyButtonId?: string;
}) {
  const t = useTranslate();

  return (
    <div
      id={applyButtonId ? `${applyButtonId}-plan` : undefined}
      className="mt-3 space-y-3"
    >
      <div className="text-sm text-muted-foreground">
        {t("rename.planSummary", {
          total: plan.total,
          renamable: plan.renamable,
          noop: plan.noop,
          conflicts: plan.conflicts,
          errors: plan.errors,
        })}
      </div>
      <div className="max-h-96 overflow-auto rounded-lg border border-border">
        <ul className="text-sm">
          {plan.items.map((item, index) => (
            <li
              key={`${item.collectionId ?? "none"}-${item.currentPath ?? ""}-${index}`}
              className="border-t border-border px-3 py-2 first:border-t-0"
            >
              {/* Shrink-wrapping the pair keeps the arrow centered on the
                  paths themselves, not the whole row. */}
              <div className="flex w-fit max-w-full flex-col items-start">
                <span className="sr-only">{t("rename.currentPath")}</span>
                <div
                  data-ui="media-rename-plan-current-path"
                  className="break-all font-[var(--font-code)] text-xs text-muted-foreground"
                >
                  {item.currentPath || "—"}
                </div>
                <ArrowDown
                  aria-hidden
                  className="my-1 h-3.5 w-3.5 self-center text-[var(--scry-accent-text)]"
                />
                <span className="sr-only">{t("rename.proposedPath")}</span>
                <div
                  data-ui="media-rename-plan-proposed-path"
                  className="break-all font-[var(--font-code)] text-xs text-card-foreground"
                >
                  {item.proposedPath ?? "—"}
                </div>
              </div>
            </li>
          ))}
        </ul>
      </div>
      <div className="flex justify-end gap-2">
        {onCancel ? (
          <Button
            id={applyButtonId ? `${applyButtonId}-cancel` : undefined}
            data-ui="media-rename-plan-cancel"
            size="sm"
            type="button"
            variant="outline"
            onClick={onCancel}
            disabled={applying}
          >
            {t("label.cancel")}
          </Button>
        ) : null}
        <Button id={applyButtonId} size="sm" type="button" onClick={onApply} disabled={applyDisabled}>
          {applying ? t("rename.applying") : t("rename.applyButton")}
        </Button>
      </div>
    </div>
  );
}

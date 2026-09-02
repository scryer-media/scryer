import { FolderInput } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";

type MoveTitleSettingsCardProps = {
  idPrefix: string;
  onOpen: () => void;
};

/**
 * The title panel's one way into the move workflow (FR-011).
 *
 * It replaces the destination-root dropdown that used to double as the trigger:
 * a library with a single root offered nothing else to pick, and reselecting the
 * current value fired nothing, so a cross-library move was unreachable from the
 * panel. The action opens the move wizard, which asks what kind of move this is
 * before it asks where.
 */
export function MoveTitleSettingsCard({
  idPrefix,
  onOpen,
}: MoveTitleSettingsCardProps) {
  const t = useTranslate();

  return (
    <div
      id={`${idPrefix}-move-to-card`}
      className="mt-5 flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-muted/20 px-3 py-3"
    >
      <div className="min-w-0">
        <p className="text-sm font-medium text-foreground">
          {t("move.actionHeading")}
        </p>
        <p className="text-xs text-muted-foreground">
          {t("move.actionDescription")}
        </p>
      </div>
      <Button
        id={`${idPrefix}-move-to`}
        type="button"
        variant="primary"
        size="sm"
        className="shrink-0"
        onClick={onOpen}
      >
        <FolderInput className="mr-2 h-4 w-4" />
        {t("move.actionButton")}
      </Button>
    </div>
  );
}

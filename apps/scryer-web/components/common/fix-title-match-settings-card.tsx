import { Search } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";

type FixTitleMatchSettingsCardProps = {
  facet: string;
  idPrefix: string;
  onOpen: () => void;
  compact?: boolean;
};

export function FixTitleMatchSettingsCard({
  facet,
  idPrefix,
  onOpen,
  compact = false,
}: FixTitleMatchSettingsCardProps) {
  const t = useTranslate();
  const descriptionKey =
    facet.trim().toUpperCase() === "MOVIE"
      ? "title.fixMatchDescriptionMovie"
      : "title.fixMatchDescriptionSeries";

  const action = (
    <Button
      id={`${idPrefix}-fix-match`}
      type="button"
      variant="primary"
      size="sm"
      className="shrink-0"
      onClick={onOpen}
    >
      <Search className="h-4 w-4" />
      {t("title.fixMatchAction")}
    </Button>
  );

  if (compact) {
    return action;
  }

  return (
    <div className="mt-3 flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-muted/20 px-3 py-3">
      <div className="min-w-0">
        <p className="text-sm font-medium text-foreground">
          {t("title.fixMatchHeading")}
        </p>
        <p className="text-xs text-muted-foreground">{t(descriptionKey)}</p>
      </div>
      {action}
    </div>
  );
}

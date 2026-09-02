import { Search } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";

type FixTitleMatchSettingsCardProps = {
  facet: string;
  idPrefix: string;
  onOpen: () => void;
};

export function FixTitleMatchSettingsCard({
  facet,
  idPrefix,
  onOpen,
}: FixTitleMatchSettingsCardProps) {
  const t = useTranslate();
  const descriptionKey =
    facet.trim().toUpperCase() === "MOVIE"
      ? "title.fixMatchDescriptionMovie"
      : "title.fixMatchDescriptionSeries";

  return (
    <div className="mt-3 flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-muted/20 px-3 py-3">
      <div className="min-w-0">
        <p className="text-sm font-medium text-foreground">
          {t("title.fixMatchHeading")}
        </p>
        <p className="text-xs text-muted-foreground">{t(descriptionKey)}</p>
      </div>
      <Button
        id={`${idPrefix}-fix-match`}
        type="button"
        variant="primary"
        size="sm"
        className="shrink-0"
        onClick={onOpen}
      >
        <Search className="mr-2 h-4 w-4" />
        {t("title.fixMatchAction")}
      </Button>
    </div>
  );
}

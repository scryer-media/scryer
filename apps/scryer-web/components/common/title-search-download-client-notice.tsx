import { AlertTriangle } from "lucide-react";
import { Link } from "react-router";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";

type Props = {
  className?: string;
};

export function TitleSearchDownloadClientNotice({ className }: Props) {
  const t = useTranslate();

  return (
    <div
      className={cn(
        "rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-4",
        className,
      )}
    >
      <div className="flex items-start gap-3">
        <AlertTriangle className="mt-0.5 h-5 w-5 shrink-0 text-[var(--scry-warning-text)]" />
        <div className="space-y-3">
          <div className="space-y-1">
            <p className="text-sm font-semibold text-foreground">
              {t("title.searchNeedsDownloadClientTitle")}
            </p>
            <p className="text-sm text-muted-foreground">
              {t("title.searchNeedsDownloadClientDescription")}
            </p>
          </div>
          <Button
            asChild
            size="sm"
            variant="outline"
            className="border-[var(--scry-warning-border)] bg-background/80"
          >
            <Link to="/settings/download-clients">
              {t("title.searchNeedsDownloadClientAction")}
            </Link>
          </Button>
        </div>
      </div>
    </div>
  );
}

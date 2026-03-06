import * as React from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { LibraryScanSummary } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";

type MediaLibrarySettingsPanelProps = {
  settingsTitle: string;
  pathLabel: string;
  pathValue: string;
  pathPlaceholder: string;
  pathHelp: string;
  pathRequired: boolean;
  onPathChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  loading: boolean;
  scanLoading: boolean;
  scanSummary: LibraryScanSummary | null;
  onScan: () => void;
};

export const MediaLibrarySettingsPanel = React.memo(function MediaLibrarySettingsPanel({
  settingsTitle,
  pathLabel,
  pathValue,
  pathPlaceholder,
  pathHelp,
  pathRequired,
  onPathChange,
  loading,
  scanLoading,
  scanSummary,
  onScan,
}: MediaLibrarySettingsPanelProps) {
  const t = useTranslate();
  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>{settingsTitle}</CardTitle>
        </CardHeader>
        <CardContent>
          <label>
            <Label className="mb-2 block">{pathLabel}</Label>
            <Input
              value={pathValue}
              onChange={onPathChange}
              placeholder={pathPlaceholder}
              required={pathRequired}
              disabled={loading}
            />
            <p className="mt-1 text-xs text-muted-foreground">
              {loading ? t("label.loading") : pathHelp}
            </p>
          </label>
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>{t("settings.libraryScanTitle")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <p className="text-sm text-muted-foreground">{t("settings.libraryScanHelp")}</p>
          <div className="flex flex-wrap items-center gap-3">
            <Button
              type="button"
              onClick={onScan}
              disabled={scanLoading}
            >
              {scanLoading
                ? t("settings.libraryScanRunning")
                : t("settings.libraryScanButton")}
            </Button>
            {scanSummary ? (
              <span className="text-xs text-muted-foreground">
                {t("settings.libraryScanSummary", {
                  imported: scanSummary.imported,
                  skipped: scanSummary.skipped,
                  unmatched: scanSummary.unmatched,
                })}
              </span>
            ) : null}
          </div>
        </CardContent>
      </Card>
    </>
  );
});

import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { ViewCategoryId } from "./indexer-category-picker";

const FILLER_POLICY_OPTIONS = [
  { value: "download_all", label: "settings.fillerPolicyDownloadAll" },
  { value: "skip_filler", label: "settings.fillerPolicySkipFiller" },
];

const RECAP_POLICY_OPTIONS = [
  { value: "download_all", label: "settings.recapPolicyDownloadAll" },
  { value: "skip_recap", label: "settings.recapPolicySkipRecap" },
];

export function GeneralSettingsPanel({
  activeQualityScopeId,
  mediaSettingsLoading,
  mediaSettingsSaving,
  categoryFillerPolicies,
  handleFillerPolicyChange,
  categoryRecapPolicies,
  handleRecapPolicyChange,
  categoryMonitorSpecials,
  handleMonitorSpecialsChange,
  categoryInterSeasonMovies,
  handleInterSeasonMoviesChange,
  nfoWriteOnImport,
  handleNfoWriteChange,
  plexmatchWriteOnImport,
  handlePlexmatchWriteChange,
  updateCategoryMediaProfileSettings,
}: {
  activeQualityScopeId: ViewCategoryId;
  mediaSettingsLoading: boolean;
  mediaSettingsSaving: boolean;
  categoryFillerPolicies: Record<ViewCategoryId, string>;
  handleFillerPolicyChange: (value: string) => void;
  categoryRecapPolicies: Record<ViewCategoryId, string>;
  handleRecapPolicyChange: (value: string) => void;
  categoryMonitorSpecials: Record<ViewCategoryId, string>;
  handleMonitorSpecialsChange: (checked: boolean) => void;
  categoryInterSeasonMovies: Record<ViewCategoryId, string>;
  handleInterSeasonMoviesChange: (checked: boolean) => void;
  nfoWriteOnImport: Record<ViewCategoryId, string>;
  handleNfoWriteChange: (checked: boolean) => void;
  plexmatchWriteOnImport: Record<ViewCategoryId, string>;
  handlePlexmatchWriteChange: (checked: boolean) => void;
  updateCategoryMediaProfileSettings: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
}) {
  const t = useTranslate();

  return (
    <form onSubmit={updateCategoryMediaProfileSettings} className="space-y-4">
      {activeQualityScopeId === "anime" && (
        <Card>
          <CardHeader>
            <CardTitle>{t("facetSettings.generalPolicies")}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            {activeQualityScopeId === "anime" && (
              <div className="grid gap-4 md:grid-cols-2">
                <label className="space-y-2">
                  <Label className="text-sm text-card-foreground">
                    {t("settings.fillerPolicyLabel")}
                  </Label>
                  <Select value={categoryFillerPolicies[activeQualityScopeId]} onValueChange={handleFillerPolicyChange} disabled={mediaSettingsLoading}>
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {FILLER_POLICY_OPTIONS.map((option) => (
                        <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </label>
                <label className="space-y-2">
                  <Label className="text-sm text-card-foreground">
                    {t("settings.recapPolicyLabel")}
                  </Label>
                  <Select value={categoryRecapPolicies[activeQualityScopeId]} onValueChange={handleRecapPolicyChange} disabled={mediaSettingsLoading}>
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {RECAP_POLICY_OPTIONS.map((option) => (
                        <SelectItem key={option.value} value={option.value}>{t(option.label)}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </label>
                <div className="space-y-2">
                  <Label className="text-sm text-card-foreground">
                    {t("settings.monitorSpecialsLabel")}
                  </Label>
                  <div className="flex items-center gap-3">
                    <button
                      type="button"
                      role="switch"
                      aria-checked={categoryMonitorSpecials[activeQualityScopeId] !== "false"}
                      className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${categoryMonitorSpecials[activeQualityScopeId] !== "false" ? "bg-primary" : "bg-muted"}`}
                      onClick={() => handleMonitorSpecialsChange(categoryMonitorSpecials[activeQualityScopeId] === "false")}
                      disabled={mediaSettingsLoading}
                    >
                      <span
                        className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${categoryMonitorSpecials[activeQualityScopeId] !== "false" ? "translate-x-5" : "translate-x-0"}`}
                      />
                    </button>
                    <span className="text-xs text-muted-foreground">{t("settings.monitorSpecialsDescription")}</span>
                  </div>
                </div>
                <div className="space-y-2">
                  <Label className="text-sm text-card-foreground">
                    {t("settings.interSeasonMoviesLabel")}
                  </Label>
                  <div className="flex items-center gap-3">
                    <button
                      type="button"
                      role="switch"
                      aria-checked={categoryInterSeasonMovies[activeQualityScopeId] !== "false"}
                      className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${categoryInterSeasonMovies[activeQualityScopeId] !== "false" ? "bg-primary" : "bg-muted"}`}
                      onClick={() => handleInterSeasonMoviesChange(categoryInterSeasonMovies[activeQualityScopeId] === "false")}
                      disabled={mediaSettingsLoading}
                    >
                      <span
                        className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${categoryInterSeasonMovies[activeQualityScopeId] !== "false" ? "translate-x-5" : "translate-x-0"}`}
                      />
                    </button>
                    <span className="text-xs text-muted-foreground">{t("settings.interSeasonMoviesDescription")}</span>
                  </div>
                </div>
              </div>
            )}
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader>
          <CardTitle>{t("facetSettings.sidecarFiles")}</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <Label className="text-sm text-card-foreground">
                {t("settings.nfoWriteOnImportLabel")}
              </Label>
              <div className="flex items-center gap-3">
                <button
                  type="button"
                  role="switch"
                  aria-checked={nfoWriteOnImport[activeQualityScopeId] === "true"}
                  className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${nfoWriteOnImport[activeQualityScopeId] === "true" ? "bg-primary" : "bg-muted"}`}
                  onClick={() => handleNfoWriteChange(nfoWriteOnImport[activeQualityScopeId] !== "true")}
                  disabled={mediaSettingsLoading}
                >
                  <span
                    className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${nfoWriteOnImport[activeQualityScopeId] === "true" ? "translate-x-5" : "translate-x-0"}`}
                  />
                </button>
                <span className="text-xs text-muted-foreground">{t("settings.nfoWriteOnImportDescription")}</span>
              </div>
            </div>
            {(activeQualityScopeId === "series" || activeQualityScopeId === "anime") && (
              <div className="space-y-2">
                <Label className="text-sm text-card-foreground">
                  {t("settings.plexmatchWriteOnImportLabel")}
                </Label>
                <div className="flex items-center gap-3">
                  <button
                    type="button"
                    role="switch"
                    aria-checked={plexmatchWriteOnImport[activeQualityScopeId] === "true"}
                    className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${plexmatchWriteOnImport[activeQualityScopeId] === "true" ? "bg-primary" : "bg-muted"}`}
                    onClick={() => handlePlexmatchWriteChange(plexmatchWriteOnImport[activeQualityScopeId] !== "true")}
                    disabled={mediaSettingsLoading}
                  >
                    <span
                      className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${plexmatchWriteOnImport[activeQualityScopeId] === "true" ? "translate-x-5" : "translate-x-0"}`}
                    />
                  </button>
                  <span className="text-xs text-muted-foreground">{t("settings.plexmatchWriteOnImportDescription")}</span>
                </div>
              </div>
            )}
          </div>
        </CardContent>
      </Card>

      <div className="flex justify-end">
        <Button type="submit" disabled={mediaSettingsSaving}>
          {mediaSettingsSaving ? t("label.saving") : t("label.save")}
        </Button>
      </div>
    </form>
  );
}

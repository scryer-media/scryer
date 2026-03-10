import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { InfoHelp } from "@/components/common/info-help";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { ViewCategoryId } from "./indexer-category-picker";
import type { ParsedQualityProfileEntry, ScoringPersonaId, ScoringOverridesPayload } from "@/lib/types/quality-profiles";
import {
  SCORING_PERSONA_CHOICES,
  PERSONA_OVERRIDE_DEFAULTS,
  PERSONA_DESCRIPTION_KEYS,
  PERSONA_SCORING_TRAITS,
} from "@/lib/constants/quality-profiles";

type ParsedQualityProfile = {
  id: string;
  name: string;
};

type QualityProfileOption = {
  value: string;
  label: string;
};

const SCORING_OVERRIDE_LABELS: Array<[keyof ScoringOverridesPayload, string]> = [
  ["allow_x265_non4k", "qualityProfile.overrideAllowX265Non4k"],
  ["block_dv_without_fallback", "qualityProfile.overrideBlockDvNoFallback"],
  ["prefer_compact_encodes", "qualityProfile.overridePreferCompact"],
  ["prefer_lossless_audio", "qualityProfile.overridePreferLossless"],
  ["block_upscaled", "qualityProfile.overrideBlockUpscaled"],
];

export function QualitySettingsPanel({
  contentSettingsLabel,
  mediaSettingsLoading,
  mediaSettingsSaving,
  qualityProfiles,
  qualityProfileEntries,
  qualityProfileParseError,
  categoryQualityProfileOverrides,
  activeQualityScopeId,
  globalQualityProfileId,
  qualityProfileInheritValue,
  toProfileOptions,
  handleQualityProfileOverrideChange,
  onFacetPersonaSave,
  updateCategoryMediaProfileSettings,
}: {
  contentSettingsLabel: string;
  mediaSettingsLoading: boolean;
  mediaSettingsSaving: boolean;
  qualityProfiles: ParsedQualityProfile[];
  qualityProfileEntries: ParsedQualityProfileEntry[];
  qualityProfileParseError: string;
  categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
  activeQualityScopeId: ViewCategoryId;
  globalQualityProfileId: string;
  qualityProfileInheritValue: string;
  toProfileOptions: (profiles: ParsedQualityProfile[]) => QualityProfileOption[];
  handleQualityProfileOverrideChange: (value: string) => void;
  onFacetPersonaSave: (persona: ScoringPersonaId | null) => Promise<void>;
  updateCategoryMediaProfileSettings: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
}) {
  const t = useTranslate();

  const effectiveProfile = React.useMemo(() => {
    const overrideId = categoryQualityProfileOverrides[activeQualityScopeId];
    const effectiveProfileId = (!overrideId || overrideId === qualityProfileInheritValue)
      ? globalQualityProfileId
      : overrideId;
    return qualityProfileEntries.find((p) => p.id === effectiveProfileId) ?? null;
  }, [categoryQualityProfileOverrides, activeQualityScopeId, qualityProfileInheritValue, globalQualityProfileId, qualityProfileEntries]);

  const storedFacetOverride = effectiveProfile?.criteria.facet_persona_overrides[activeQualityScopeId] ?? null;
  const [selectedPersona, setSelectedPersona] = React.useState<string>(
    storedFacetOverride ?? "__default__",
  );

  // Reset local state when scope or profile changes
  React.useEffect(() => {
    const override = effectiveProfile?.criteria.facet_persona_overrides[activeQualityScopeId] ?? null;
    setSelectedPersona(override ?? "__default__");
  }, [effectiveProfile, activeQualityScopeId]);

  const resolvedPersona: ScoringPersonaId = React.useMemo(() => {
    if (selectedPersona !== "__default__") return selectedPersona as ScoringPersonaId;
    return effectiveProfile?.criteria.scoring_persona ?? "Balanced";
  }, [selectedPersona, effectiveProfile]);

  const [personaSaving, setPersonaSaving] = React.useState(false);

  const handleSubmit = React.useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      setPersonaSaving(true);
      try {
        const personaValue = selectedPersona === "__default__" ? null : (selectedPersona as ScoringPersonaId);
        await onFacetPersonaSave(personaValue);
        await updateCategoryMediaProfileSettings(event);
      } finally {
        setPersonaSaving(false);
      }
    },
    [selectedPersona, onFacetPersonaSave, updateCategoryMediaProfileSettings],
  );

  const isSaving = mediaSettingsSaving || personaSaving;
  const traits = PERSONA_SCORING_TRAITS[resolvedPersona];

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>{t("settings.qualityProfileSection")}</CardTitle>
        </CardHeader>
        <CardContent>
          <label>
            <Label className="mb-2 inline-flex items-center gap-2">
              {t("settings.qualityProfileOverrideLabel", {
                category: contentSettingsLabel.toLowerCase(),
              })}
              <InfoHelp
                text={t("settings.qualityProfileOverrideHelp")}
                ariaLabel={t("settings.qualityProfileOverrideHelp")}
              />
            </Label>
            <Select value={categoryQualityProfileOverrides[activeQualityScopeId]} onValueChange={handleQualityProfileOverrideChange} disabled={mediaSettingsLoading}>
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={qualityProfileInheritValue}>{t("settings.qualityProfileInheritLabel")}</SelectItem>
                {toProfileOptions(qualityProfiles).map((opt) => (
                  <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            {qualityProfileParseError ? (
              <p className="mt-2 rounded border border-rose-500/60 bg-rose-500/10 p-2 text-xs text-rose-300">
                {qualityProfileParseError}
              </p>
            ) : null}
          </label>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("facetSettings.scoringPersona")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="space-y-2">
            <label>
              <Label className="mb-2 inline-flex items-center gap-2">
                {t("facetSettings.scoringPersonaOverrideLabel")}
              </Label>
              <Select
                value={selectedPersona}
                onValueChange={setSelectedPersona}
                disabled={mediaSettingsLoading || !effectiveProfile}
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__default__">
                    {t("facetSettings.scoringPersonaUseDefault")}
                    {effectiveProfile ? ` (${effectiveProfile.criteria.scoring_persona})` : ""}
                  </SelectItem>
                  {SCORING_PERSONA_CHOICES.map((choice) => (
                    <SelectItem key={choice.value} value={choice.value}>
                      {t(choice.labelKey)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            <p className="text-sm text-muted-foreground">
              {t(PERSONA_DESCRIPTION_KEYS[resolvedPersona])}
            </p>
          </div>

          <div className="space-y-2.5">
            <Label className="inline-flex items-center gap-2 text-sm text-card-foreground">
              {t("facetSettings.scoringBehavior")}
              <InfoHelp
                text={t("facetSettings.scoringBehaviorHint")}
                ariaLabel={t("facetSettings.scoringBehavior")}
              />
            </Label>
            <ul className="grid gap-1 sm:grid-cols-2">
              {traits.map((traitKey) => (
                <li key={traitKey} className="text-xs text-muted-foreground">
                  {t(traitKey)}
                </li>
              ))}
            </ul>
          </div>

          <details className="rounded-lg border border-border/50 p-2">
            <summary className="cursor-pointer select-none text-xs font-medium text-muted-foreground">
              <span className="inline-flex items-center gap-2">
                {t("facetSettings.effectiveScoringOverrides")}
                <InfoHelp
                  text={t("facetSettings.effectiveScoringOverridesHint")}
                  ariaLabel={t("facetSettings.effectiveScoringOverrides")}
                />
              </span>
            </summary>
            <div className="mt-3 grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
              {SCORING_OVERRIDE_LABELS.map(([key, labelKey]) => {
                const explicitValue = effectiveProfile?.criteria.scoring_overrides[key];
                const personaDefault = PERSONA_OVERRIDE_DEFAULTS[resolvedPersona]?.[key] ?? false;
                const effectiveValue = explicitValue ?? personaDefault;
                return (
                  <div key={key} className="flex items-center gap-2">
                    <RenderBooleanIcon value={effectiveValue} label={t(labelKey)} />
                    <span className="text-xs text-muted-foreground">{t(labelKey)}</span>
                  </div>
                );
              })}
            </div>
          </details>
        </CardContent>
      </Card>

      <div className="flex justify-end">
        <Button type="submit" disabled={isSaving}>
          {isSaving ? t("label.saving") : t("label.save")}
        </Button>
      </div>
    </form>
  );
}

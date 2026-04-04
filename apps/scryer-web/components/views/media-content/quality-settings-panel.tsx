import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { InfoHelp } from "@/components/common/info-help";
import { SubtitleLanguagePicker } from "@/components/common/subtitle-language-picker";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { ViewCategoryId } from "./indexer-category-picker";
import type {
  FacetScoringPersonaSelectionRecord,
  ScoringPersonaId,
} from "@/lib/types/quality-profiles";
import {
  PERSONA_DESCRIPTION_KEYS,
  PERSONA_SCORING_TRAITS,
  SCORING_PERSONA_CHOICES,
} from "@/lib/constants/quality-profiles";

type ParsedQualityProfile = {
  id: string;
  name: string;
};

type QualityProfileOption = {
  value: string;
  label: string;
};

export function QualitySettingsPanel({
  contentSettingsLabel,
  mediaSettingsLoading,
  mediaSettingsSaving,
  qualityProfiles,
  qualityProfileParseError,
  categoryQualityProfileOverrides,
  categoryRequiredAudioLanguages,
  saveCategoryRequiredAudioLanguages,
  activeQualityScopeId,
  globalScoringPersona,
  categoryPersonaSelections,
  qualityProfileInheritValue,
  toProfileOptions,
  saveCategoryQualityProfileOverride,
  onFacetPersonaSave,
}: {
  contentSettingsLabel: string;
  mediaSettingsLoading: boolean;
  mediaSettingsSaving: boolean;
  qualityProfiles: ParsedQualityProfile[];
  qualityProfileParseError: string;
  categoryQualityProfileOverrides: Record<ViewCategoryId, string>;
  categoryRequiredAudioLanguages: Record<ViewCategoryId, string[]>;
  saveCategoryRequiredAudioLanguages: (languages: string[]) => Promise<void> | void;
  activeQualityScopeId: ViewCategoryId;
  globalScoringPersona: ScoringPersonaId;
  categoryPersonaSelections: Record<ViewCategoryId, FacetScoringPersonaSelectionRecord>;
  qualityProfileInheritValue: string;
  toProfileOptions: (profiles: ParsedQualityProfile[]) => QualityProfileOption[];
  saveCategoryQualityProfileOverride: (value: string) => Promise<void> | void;
  onFacetPersonaSave: (persona: ScoringPersonaId | null) => Promise<void> | void;
}) {
  const t = useTranslate();
  const personaSelection =
    categoryPersonaSelections[activeQualityScopeId] ?? {
      scope: activeQualityScopeId,
      overridePersona: null,
      effectivePersona: globalScoringPersona,
      inheritsGlobal: true,
    };
  const basePersonaSelection = personaSelection.overridePersona ?? "__default__";
  const [selectedPersona, setSelectedPersona] = React.useState<string>(
    basePersonaSelection,
  );

  React.useEffect(() => {
    setSelectedPersona(basePersonaSelection);
  }, [activeQualityScopeId, basePersonaSelection]);

  const resolvedPersona: ScoringPersonaId = React.useMemo(() => {
    if (selectedPersona !== "__default__") return selectedPersona as ScoringPersonaId;
    return personaSelection.effectivePersona ?? globalScoringPersona;
  }, [globalScoringPersona, personaSelection.effectivePersona, selectedPersona]);

  const [personaSaving, setPersonaSaving] = React.useState(false);
  const traits = PERSONA_SCORING_TRAITS[resolvedPersona];

  return (
    <div className="space-y-4">
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
            <Select
              value={categoryQualityProfileOverrides[activeQualityScopeId]}
              onValueChange={(value) => {
                void saveCategoryQualityProfileOverride(value);
              }}
              disabled={mediaSettingsLoading || mediaSettingsSaving || personaSaving}
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={qualityProfileInheritValue}>
                  {t("settings.qualityProfileInheritLabel")}
                </SelectItem>
                {toProfileOptions(qualityProfiles).map((opt) => (
                  <SelectItem key={opt.value} value={opt.value}>
                    {opt.label}
                  </SelectItem>
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
                onValueChange={async (value) => {
                  if (value === selectedPersona) {
                    return;
                  }
                  const previousValue = selectedPersona;
                  setSelectedPersona(value);
                  setPersonaSaving(true);
                  try {
                    const personaValue =
                      value === "__default__" ? null : (value as ScoringPersonaId);
                    await onFacetPersonaSave(personaValue);
                  } catch {
                    setSelectedPersona(previousValue);
                  } finally {
                    setPersonaSaving(false);
                  }
                }}
                disabled={mediaSettingsLoading || mediaSettingsSaving || personaSaving}
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__default__">
                    {t("facetSettings.scoringPersonaUseDefault")}
                    {` (${globalScoringPersona})`}
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

          <div className="space-y-2">
            <Label className="inline-flex items-center gap-2 text-sm text-card-foreground">
              {t("title.requiredAudioLanguages")}
              <InfoHelp
                text={t("title.requiredAudioLanguagesFacetInfo")}
                ariaLabel={t("title.requiredAudioLanguages")}
              />
            </Label>
            <SubtitleLanguagePicker
              value={categoryRequiredAudioLanguages[activeQualityScopeId] ?? []}
              onChange={(languages) => {
                void saveCategoryRequiredAudioLanguages(languages);
              }}
              disabled={mediaSettingsLoading || mediaSettingsSaving || personaSaving}
            />
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

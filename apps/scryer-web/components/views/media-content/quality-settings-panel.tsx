import * as React from "react";
import { Languages, SlidersVertical, Sparkles } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import { InfoHelp } from "@/components/common/info-help";
import { AudioLanguagePicker } from "@/components/common/audio-language-picker";
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

type SectionIcon = React.ComponentType<{ className?: string }>;

function QualitySettingsSection({
  icon: Icon,
  title,
  description,
  children,
}: {
  icon: SectionIcon;
  title: string;
  description?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-[16px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-5 sm:p-6">
      <div className="flex items-center gap-2.5">
        <Icon className="h-[17px] w-[17px] text-[var(--scry-accent-text)]" />
        <h2 className="text-[16px] font-bold text-[var(--scry-ink2)]">{title}</h2>
      </div>
      {description ? (
        <p className="mt-1.5 text-[12.5px] leading-relaxed text-[var(--scry-muted3)]">
          {description}
        </p>
      ) : null}
      <div className="mt-5 space-y-5">{children}</div>
    </section>
  );
}

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
  const globalScoringPersonaLabelKey =
    SCORING_PERSONA_CHOICES.find((choice) => choice.value === globalScoringPersona)
      ?.labelKey ?? "qualityProfile.personaBalanced";

  return (
    <div className="space-y-[18px]">
      <QualitySettingsSection
        icon={SlidersVertical}
        title={t("settings.qualityProfileSection")}
        description={t("settings.qualityProfileOverrideHelp")}
      >
        <div className="max-w-xl space-y-2">
          <Label>
            {t("settings.qualityProfileOverrideLabel", {
              category: contentSettingsLabel.toLowerCase(),
            })}
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
            <p className="rounded-[12px] border border-[var(--scry-danger-border-strong)] bg-[var(--scry-danger-bg)] p-3 text-xs text-[var(--scry-danger-text)]">
              {qualityProfileParseError}
            </p>
          ) : null}
        </div>
      </QualitySettingsSection>

      <QualitySettingsSection
        icon={Sparkles}
        title={t("facetSettings.scoringPersona")}
        description={t(PERSONA_DESCRIPTION_KEYS[resolvedPersona])}
      >
        <div className="space-y-5">
          <div className="max-w-xl space-y-2">
            <Label>
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
                  {` (${t(globalScoringPersonaLabelKey)})`}
                </SelectItem>
                {SCORING_PERSONA_CHOICES.map((choice) => (
                  <SelectItem key={choice.value} value={choice.value}>
                    {t(choice.labelKey)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)] p-4">
            <Label className="inline-flex items-center gap-2 text-[13.5px] font-semibold text-[var(--scry-body)]">
              {t("facetSettings.scoringBehavior")}
              <InfoHelp
                text={t("facetSettings.scoringBehaviorHint")}
                ariaLabel={t("facetSettings.scoringBehavior")}
              />
            </Label>
            <ul className="mt-3 grid gap-2 sm:grid-cols-2">
              {traits.map((traitKey) => (
                <li
                  key={traitKey}
                  className="rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-bg)] px-3 py-2 text-xs leading-relaxed text-[var(--scry-text2)]"
                >
                  {t(traitKey)}
                </li>
              ))}
            </ul>
          </div>
        </div>
      </QualitySettingsSection>

      <QualitySettingsSection
        icon={Languages}
        title={t("title.requiredAudioLanguages")}
        description={t("title.requiredAudioLanguagesFacetInfo")}
      >
        <div className="max-w-xl space-y-2">
          <AudioLanguagePicker
            value={categoryRequiredAudioLanguages[activeQualityScopeId] ?? []}
            onChange={(languages) => {
              void saveCategoryRequiredAudioLanguages(languages);
            }}
            disabled={mediaSettingsLoading || mediaSettingsSaving || personaSaving}
          />
        </div>
      </QualitySettingsSection>
    </div>
  );
}

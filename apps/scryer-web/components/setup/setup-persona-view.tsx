import {
  Headphones,
  Scale,
  SlidersHorizontal,
  Zap,
  MonitorSmartphone,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  SetupBackButton,
  SetupPanel,
  SetupPrimaryButton,
  SetupStepHeader,
} from "./setup-chrome";
import type {
  ScoringPersonaId,
  QualityTargetId,
  FacetQualityPrefs,
  ViewCategoryId,
} from "@/lib/types/quality-profiles";
import { selectorToken } from "@/lib/utils/dom-ids";

interface SetupPersonaViewProps {
  t: (key: string) => string;
  facetPrefs: Record<ViewCategoryId, FacetQualityPrefs>;
  onFacetPrefsChange: (facet: ViewCategoryId, prefs: FacetQualityPrefs) => void;
  onNext: () => void;
  onBack: () => void;
  onSkip?: () => void;
  saving: boolean;
}

const PERSONAS: {
  id: ScoringPersonaId;
  icon: typeof Scale;
  labelKey: string;
  descKey: string;
}[] = [
  {
    id: "BALANCED",
    icon: Scale,
    labelKey: "qualityProfile.personaBalanced",
    descKey: "setup.personaBalancedDesc",
  },
  {
    id: "AUDIOPHILE",
    icon: Headphones,
    labelKey: "qualityProfile.personaAudiophile",
    descKey: "setup.personaAudiophileDesc",
  },
  {
    id: "EFFICIENT",
    icon: Zap,
    labelKey: "qualityProfile.personaEfficient",
    descKey: "setup.personaEfficientDesc",
  },
  {
    id: "COMPATIBLE",
    icon: MonitorSmartphone,
    labelKey: "qualityProfile.personaCompatible",
    descKey: "setup.personaCompatibleDesc",
  },
];

const QUALITY_TARGETS: QualityTargetId[] = ["1080p", "4k"];

const FACETS: { id: ViewCategoryId; labelKey: string }[] = [
  { id: "MOVIE", labelKey: "setup.facetMovies" },
  { id: "SERIES", labelKey: "setup.facetSeries" },
  { id: "ANIME", labelKey: "setup.facetAnime" },
];

export function SetupPersonaView({
  t,
  facetPrefs,
  onFacetPrefsChange,
  onNext,
  onBack,
  onSkip,
  saving,
}: SetupPersonaViewProps) {
  return (
    <SetupPanel id="setup-persona-view" className="flex flex-col gap-6">
      <SetupStepHeader
        icon={SlidersHorizontal}
        title={t("setup.personaTitle")}
        subtitle={t("setup.personaDescription")}
      />

      {/* Persona reference */}
      <div className="grid grid-cols-2 gap-x-6 gap-y-1 rounded-lg border border-border bg-muted/30 px-4 py-3 text-xs text-muted-foreground">
        {PERSONAS.map(({ id, icon: Icon, labelKey, descKey }) => (
          <div key={id} className="flex items-start gap-1.5 py-0.5">
            <Icon className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>
              <span className="font-medium text-foreground">{t(labelKey)}</span>{" "}
              — {t(descKey)}
            </span>
          </div>
        ))}
      </div>

      {/* Per-facet selection */}
      <div className="space-y-3">
        {FACETS.map(({ id: facet, labelKey }) => {
          const prefs = facetPrefs[facet];
          return (
            <div key={facet} className="rounded-lg border border-border p-4">
              <h3 className="mb-3 text-sm font-medium">{t(labelKey)}</h3>
              <div className="flex flex-wrap items-start gap-4">
                {/* Quality target */}
                <div>
                  <span className="mb-1.5 block text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                    {t("setup.qualityTarget")}
                  </span>
                  <div className="flex gap-1">
                    {QUALITY_TARGETS.map((q) => (
                      <button
                        id={`setup-persona-${selectorToken(facet)}-quality-${q}`}
                        key={q}
                        type="button"
                        onClick={() =>
                          onFacetPrefsChange(facet, { ...prefs, quality: q })
                        }
                        className={`rounded-md border px-3 py-1.5 text-sm font-medium transition-colors ${
                          prefs.quality === q
                            ? "border-primary bg-primary text-primary-foreground"
                            : "border-border bg-background text-foreground hover:bg-muted"
                        }`}
                      >
                        {formatQualityTarget(q)}
                      </button>
                    ))}
                  </div>
                </div>

                {/* Persona */}
                <div className="flex-1">
                  <span className="mb-1.5 block text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
                    {t("setup.scoringFocus")}
                  </span>
                  <div className="flex flex-wrap gap-1">
                    {PERSONAS.map(({ id: persona, icon: Icon, labelKey }) => (
                      <button
                        id={`setup-persona-${selectorToken(facet)}-persona-${selectorToken(persona)}`}
                        key={persona}
                        type="button"
                        onClick={() =>
                          onFacetPrefsChange(facet, { ...prefs, persona })
                        }
                        className={`inline-flex items-center gap-1.5 rounded-md border px-3 py-1.5 text-sm font-medium transition-colors ${
                          prefs.persona === persona
                            ? "border-primary bg-primary text-primary-foreground"
                            : "border-border bg-background text-foreground hover:bg-muted"
                        }`}
                      >
                        <Icon className="h-3.5 w-3.5" />
                        {t(labelKey)}
                      </button>
                    ))}
                  </div>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      <div className="flex items-center justify-between pt-2">
        <SetupBackButton id="setup-persona-back" onClick={onBack}>
          {t("setup.back")}
        </SetupBackButton>
        <div className="flex items-center gap-3">
          {onSkip && (
            <Button
              id="setup-persona-skip"
              type="button"
              variant="link"
              onClick={onSkip}
              className="px-0 text-muted-foreground"
            >
              {t("setup.skip")}
            </Button>
          )}
          <SetupPrimaryButton
            id="setup-persona-next"
            onClick={onNext}
            disabled={saving}
          >
            {saving ? t("label.saving") : t("setup.next")}
          </SetupPrimaryButton>
        </div>
      </div>
    </SetupPanel>
  );
}

function formatQualityTarget(target: QualityTargetId): string {
  switch (target) {
    case "8k":
      return "8K";
    case "4k":
      return "4K";
    case "1080p":
      return "1080P";
  }
}

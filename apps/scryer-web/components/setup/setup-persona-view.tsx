import { Headphones, Scale, Zap, MonitorSmartphone } from "lucide-react";
import { Button } from "@/components/ui/button";
import type {
  ScoringPersonaId,
  QualityTargetId,
  FacetQualityPrefs,
  ViewCategoryId,
} from "@/lib/types/quality-profiles";

interface SetupPersonaViewProps {
  t: (key: string) => string;
  facetPrefs: Record<ViewCategoryId, FacetQualityPrefs>;
  onFacetPrefsChange: (facet: ViewCategoryId, prefs: FacetQualityPrefs) => void;
  onNext: () => void;
  onBack: () => void;
  onSkip?: () => void;
  saving: boolean;
}

const PERSONAS: { id: ScoringPersonaId; icon: typeof Scale; descKey: string }[] = [
  { id: "Balanced", icon: Scale, descKey: "setup.personaBalancedDesc" },
  { id: "Audiophile", icon: Headphones, descKey: "setup.personaAudiophileDesc" },
  { id: "Efficient", icon: Zap, descKey: "setup.personaEfficientDesc" },
  { id: "Compatible", icon: MonitorSmartphone, descKey: "setup.personaCompatibleDesc" },
];

const QUALITY_TARGETS: QualityTargetId[] = ["4k", "1080p"];

const FACETS: { id: ViewCategoryId; labelKey: string }[] = [
  { id: "movie", labelKey: "setup.facetMovies" },
  { id: "series", labelKey: "setup.facetSeries" },
  { id: "anime", labelKey: "setup.facetAnime" },
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
    <div className="flex flex-col gap-6">
      <div className="text-center">
        <h2 className="text-xl font-semibold">{t("setup.personaTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {t("setup.personaDescription")}
        </p>
      </div>

      {/* Persona reference */}
      <div className="grid grid-cols-2 gap-x-6 gap-y-1 rounded-lg border border-border bg-muted/30 px-4 py-3 text-xs text-muted-foreground">
        {PERSONAS.map(({ id, icon: Icon, descKey }) => (
          <div key={id} className="flex items-start gap-1.5 py-0.5">
            <Icon className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>
              <span className="font-medium text-foreground">
                {t(`qualityProfile.persona${id}`)}
              </span>{" "}
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
            <div
              key={facet}
              className="rounded-lg border border-border p-4"
            >
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
                        {q === "4k" ? "4K" : "1080P"}
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
                    {PERSONAS.map(({ id: persona, icon: Icon }) => (
                      <button
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
                        {t(`qualityProfile.persona${persona}`)}
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
        <Button variant="ghost" onClick={onBack}>
          {t("setup.back")}
        </Button>
        <div className="flex items-center gap-3">
          {onSkip && (
            <button type="button" onClick={onSkip} className="text-sm text-muted-foreground underline-offset-4 hover:underline">
              {t("setup.skip")}
            </button>
          )}
          <Button onClick={onNext} disabled={saving}>
            {saving ? t("label.saving") : t("setup.next")}
          </Button>
        </div>
      </div>
    </div>
  );
}

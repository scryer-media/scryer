import { Library } from "lucide-react";

import {
  SCORING_PERSONA_VALUES,
  type ScoringPersonaValue,
  type UseExternalImportSetupReturn,
} from "@/lib/hooks/use-external-import-setup";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  facetLabelKey,
  facetPillStyle,
  facetStyle,
} from "@/components/setup/import/facet-style";
import { selectorId } from "@/lib/utils/dom-ids";

interface SetupImportQualityViewProps {
  wizard: UseExternalImportSetupReturn;
  t: (key: string, values?: Record<string, unknown>) => string;
}

/**
 * Quality & persona step body (design-spec §7). One row per mapped library: its
 * facet identity, a quality-profile select, and a scoring-persona select.
 * Container chrome (brand/stepper/header/actions) is rendered by the wizard
 * container — this renders only the step body.
 */
export default function SetupImportQualityView({
  wizard,
  t,
}: SetupImportQualityViewProps) {
  const {
    mappedLibraries,
    qualityProfiles,
    setLibraryQualityProfile,
    setLibraryPersona,
  } = wizard;

  if (mappedLibraries.length === 0) {
    return (
      <div
        id="setup-import-quality-view"
        data-slot="setup-import-quality-empty"
        style={{ marginTop: 24 }}
        className="rounded-2xl border border-border px-5 py-10 text-center text-sm"
      >
        <span style={{ color: "var(--scry-muted3)" }}>
          {t("setup.qualityNoLibraries")}
        </span>
      </div>
    );
  }

  return (
    <div
      id="setup-import-quality-view"
      data-slot="setup-import-quality-view"
      className="rounded-2xl border border-border"
      style={{
        marginTop: 24,
        background: "rgba(10, 17, 32, 0.5)",
        padding: "14px 22px",
      }}
    >
      {/* Column header (hidden on narrow screens) */}
      <div
        data-slot="setup-import-quality-head"
        className="hidden items-center gap-3 pb-1 md:flex"
        style={{
          fontSize: 10.5,
          fontWeight: 700,
          letterSpacing: "0.04em",
          textTransform: "uppercase",
          color: "var(--scry-faint3)",
        }}
      >
        <span style={{ flex: 1 }}>{t("setup.library")}</span>
        <span style={{ width: 184 }}>{t("setup.qualityProfile")}</span>
        <span style={{ width: 238 }}>{t("setup.persona")}</span>
      </div>

      {mappedLibraries.map((lib) => {
        const style = facetStyle(lib.facet);
        return (
          <div
            key={lib.id}
            id={selectorId("setup-import-quality-row", lib.id)}
            data-slot="setup-import-quality-row"
            className="flex flex-wrap items-center gap-3 md:flex-nowrap"
            style={{
              borderTop: "1px solid var(--scry-hover)",
              padding: "13px 0",
            }}
          >
            {/* Library identity */}
            <div
              className="flex min-w-0 items-center gap-3"
              style={{ flex: 1 }}
            >
              <span
                aria-hidden
                className="flex shrink-0 items-center justify-center"
                style={{
                  width: 34,
                  height: 34,
                  borderRadius: 8,
                  background: style.bg,
                  border: `1px solid ${style.border}`,
                  color: style.text,
                }}
              >
                <Library size={16} />
              </span>
              <div className="flex min-w-0 items-center gap-2">
                <span
                  className="truncate"
                  style={{
                    fontSize: 14,
                    fontWeight: 600,
                    color: "#f1f5ff",
                  }}
                  title={lib.name}
                >
                  {lib.name}
                </span>
                <span
                  className="inline-flex shrink-0 items-center gap-1.5"
                  style={{
                    padding: "3px 9px",
                    borderRadius: 7,
                    fontSize: 10,
                    fontWeight: 700,
                    whiteSpace: "nowrap",
                    ...facetPillStyle(lib.facet),
                  }}
                >
                  <span
                    aria-hidden
                    style={{
                      width: 6,
                      height: 6,
                      borderRadius: "50%",
                      background: style.dot,
                    }}
                  />
                  {t(facetLabelKey(lib.facet))}
                </span>
              </div>
            </div>

            {/* Quality profile select */}
            <div className="w-full md:w-[184px]">
              <Select
                value={lib.qualityProfileId ?? ""}
                onValueChange={(value) =>
                  setLibraryQualityProfile(lib.id, value)
                }
              >
                <SelectTrigger
                  id={selectorId("setup-import-quality-profile", lib.id)}
                  data-slot="setup-import-quality-profile-trigger"
                  className="w-full text-left"
                  style={{ minHeight: 56 }}
                >
                  <SelectValue placeholder={t("setup.qualityProfile")} />
                </SelectTrigger>
                <SelectContent position="popper">
                  {qualityProfiles.map((profile) => (
                    <SelectItem key={profile.id} value={profile.id}>
                      {profile.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            {/* Persona select */}
            <div className="w-full md:w-[238px]">
              <Select
                value={lib.scoringPersona}
                onValueChange={(value) =>
                  setLibraryPersona(lib.id, value as ScoringPersonaValue)
                }
              >
                <SelectTrigger
                  id={selectorId("setup-import-persona", lib.id)}
                  data-slot="setup-import-persona-trigger"
                  className="w-full text-left"
                  style={{
                    minHeight: 56,
                    background: "rgba(var(--scry-accent-rgb), 0.06)",
                    borderColor: "var(--scry-baccent)",
                  }}
                >
                  {/* Custom left-aligned value — SelectValue centers multi-line
                      content awkwardly, so render the selected persona directly. */}
                  <span className="flex min-w-0 flex-1 flex-col text-left">
                    <span
                      style={{
                        fontSize: 13,
                        fontWeight: 600,
                        color: "#f1f5ff",
                      }}
                    >
                      {t(
                        `setup.persona.${lib.scoringPersona.toLowerCase()}.name`,
                      )}
                    </span>
                    <span
                      className="truncate"
                      style={{ fontSize: 11, color: "var(--scry-muted3)" }}
                    >
                      {t(
                        `setup.persona.${lib.scoringPersona.toLowerCase()}.desc`,
                      )}
                    </span>
                  </span>
                </SelectTrigger>
                <SelectContent position="popper">
                  {SCORING_PERSONA_VALUES.map((persona) => (
                    <SelectItem key={persona} value={persona}>
                      <span className="flex min-w-0 flex-col">
                        <span style={{ fontSize: 13, fontWeight: 600 }}>
                          {t(`setup.persona.${persona.toLowerCase()}.name`)}
                        </span>
                        <span
                          className="truncate"
                          style={{
                            fontSize: 11,
                            color: "var(--scry-muted3)",
                          }}
                        >
                          {t(`setup.persona.${persona.toLowerCase()}.desc`)}
                        </span>
                      </span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>
        );
      })}
    </div>
  );
}

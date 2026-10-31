import { Check } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useUiSettings } from "@/lib/context/ui-settings-context";
import { normalizePalette, PALETTES } from "@/lib/palettes";
import { cn } from "@/lib/utils";

/**
 * `idPrefix` keeps element ids unique when the sidebar popover and the
 * profile settings page both mount a picker at once.
 */
export function PalettePicker({ idPrefix }: { idPrefix: string }) {
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const {
    appearanceSettings: uiSettings,
    uiSettingsLoaded,
    uiSettingsLoading,
    appearanceSaving,
    changeAppearance,
  } = useUiSettings();
  const disabled = !uiSettingsLoaded || uiSettingsLoading || appearanceSaving;
  const selectedPalette = normalizePalette(uiSettings.palette);
  const select = async (change: Parameters<typeof changeAppearance>[0]) => {
    if ((await changeAppearance(change)) === "failed") {
      setGlobalStatus(t("palette.saveFailed"), { level: "ERROR" });
    }
  };
  return (
    <div className="@container space-y-3" aria-busy={appearanceSaving}>
      <div
        role="group"
        aria-label={t("palette.title")}
        className="grid grid-cols-2 gap-2 @min-[32rem]:grid-cols-4"
      >
        {PALETTES.map((palette) => (
          <button
            key={palette.id}
            id={`${idPrefix}-option-${palette.id.toLowerCase()}`}
            type="button"
            aria-pressed={selectedPalette === palette.id}
            disabled={disabled}
            onClick={() => void select({ palette: palette.id })}
            className={cn(
              "rounded-lg border p-2 text-left text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-wait",
              selectedPalette === palette.id
                ? "border-primary bg-accent"
                : "border-border hover:bg-muted",
            )}
          >
            <span
              aria-hidden="true"
              className="mb-1.5 flex h-12 overflow-hidden rounded border border-black/10"
              style={{ background: palette.background, color: palette.ink }}
            >
              <span
                className="flex w-1/4 flex-col gap-1 p-1"
                style={{ background: palette.surface }}
              >
                <span
                  className="mt-1 h-1 rounded"
                  style={{ background: palette.accent }}
                />
                <span className="h-1 rounded bg-current opacity-30" />
                <span className="h-1 rounded bg-current opacity-30" />
              </span>
              <span className="flex flex-1 flex-col gap-1 p-1.5">
                <span className="h-1 w-2/3 rounded bg-current" />
                <span
                  className="flex-1 rounded"
                  style={{ background: palette.surface }}
                />
                <span
                  className="h-2 w-1/2 rounded"
                  style={{ background: palette.accent }}
                />
              </span>
            </span>
            <span className="flex items-center justify-between gap-1 text-xs font-medium">
              {t(palette.labelKey)}
              {selectedPalette === palette.id && (
                <Check className="size-3.5 shrink-0" aria-hidden="true" />
              )}
            </span>
          </button>
        ))}
      </div>
      {selectedPalette === "CLASSIC" && (
        <div
          role="group"
          aria-label={t("profile.appearance")}
          className="flex flex-wrap gap-2"
        >
          {(
            [
              ["LIGHT", "theme.light"],
              ["DARK", "theme.dark"],
              ["SYSTEM", "theme.system"],
            ] as const
          ).map(([theme, label]) => (
            <button
              key={theme}
              id={`${idPrefix}-theme-${theme.toLowerCase()}`}
              type="button"
              disabled={disabled}
              aria-pressed={uiSettings.theme === theme}
              onClick={() => void select({ theme })}
              className={cn(
                "rounded-md border px-3 py-1.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring",
                uiSettings.theme === theme
                  ? "border-primary bg-accent text-accent-foreground"
                  : "border-border text-foreground",
              )}
            >
              {t(label)}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

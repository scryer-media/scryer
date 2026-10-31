import { lazy, Suspense, useEffect, useState } from "react";
import { Check, Palette } from "lucide-react";
import { useTheme } from "next-themes";
import { Button } from "@/components/ui/button";
import { LoadingMark } from "@/components/common/loading-mark";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useUiSettings } from "@/lib/context/ui-settings-context";
import { AUTH_SESSION_CHANGED_EVENT } from "@/lib/hooks/use-auth";
import { getPalette } from "@/lib/palettes";
import {
  applyHighlightColor,
  HIGHLIGHT_COLOR_PRESETS,
  isDarkTheme,
} from "@/lib/theme";

const HighlightColorPicker = lazy(() => import("./highlight-color-picker"));

export function QuickHighlightColorPicker() {
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const { resolvedTheme, theme } = useTheme();
  const {
    appearanceSettings,
    uiSettingsLoaded,
    uiSettingsLoading,
    appearanceSaving,
    changeAppearance,
  } = useUiSettings();
  const [customOpen, setCustomOpen] = useState(false);
  const [preview, setPreview] = useState<string | null>(null);
  const { highlightColor, palette } = appearanceSettings;
  const selectedColor = preview ?? highlightColor ?? getPalette(palette).accent;
  const dark = isDarkTheme(resolvedTheme ?? theme);
  const disabled = !uiSettingsLoaded || uiSettingsLoading || appearanceSaving;

  useEffect(() => {
    if (preview === null) return;
    applyHighlightColor(document.documentElement, preview, dark);
    return () =>
      applyHighlightColor(document.documentElement, highlightColor, dark);
  }, [preview, highlightColor, dark]);

  useEffect(() => {
    const resetPreview = () => {
      setPreview(null);
      setCustomOpen(false);
    };
    window.addEventListener(AUTH_SESSION_CHANGED_EVENT, resetPreview);
    return () =>
      window.removeEventListener(AUTH_SESSION_CHANGED_EVENT, resetPreview);
  }, []);

  const closeCustom = () => {
    setPreview(null);
    setCustomOpen(false);
  };
  const select = async (value: string | null) => {
    closeCustom();
    if ((await changeAppearance({ highlightColor: value })) === "failed") {
      setGlobalStatus(t("palette.saveFailed"), { level: "ERROR" });
    }
  };

  return (
    <section
      className="mt-4 space-y-3 border-t border-border pt-4"
      aria-busy={appearanceSaving}
    >
      <h3 className="text-sm font-semibold">{t("profile.highlightColor")}</h3>
      <div
        role="group"
        aria-label={t("profile.highlightColor")}
        className="flex flex-wrap items-center gap-2"
      >
        {HIGHLIGHT_COLOR_PRESETS.map((preset) => (
          <button
            key={preset.value}
            type="button"
            aria-label={t(preset.labelKey)}
            title={t(preset.labelKey)}
            aria-pressed={selectedColor === preset.value}
            disabled={disabled}
            onClick={() => void select(preset.value)}
            className="relative size-8 shrink-0 rounded-[9px] outline-none transition hover:scale-105 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-wait disabled:opacity-60"
            style={{ backgroundColor: preset.value }}
          >
            {selectedColor === preset.value && (
              <Check className="absolute inset-0 m-auto size-4 text-white drop-shadow-[0_1px_2px_rgba(0,0,0,0.8)]" />
            )}
          </button>
        ))}
        <div className="flex shrink-0 items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled || highlightColor === null}
            onClick={() => void select(null)}
          >
            {t("label.default")}
          </Button>
          <Button
            type="button"
            size="icon-sm"
            variant="outline"
            disabled={disabled}
            aria-expanded={customOpen}
            aria-label={t("profile.highlightColorCustom")}
            title={t("profile.highlightColorCustom")}
            aria-controls={
              customOpen ? "theme-quick-custom-highlight" : undefined
            }
            onClick={() => (customOpen ? closeCustom() : setCustomOpen(true))}
          >
            <Palette className="size-4" aria-hidden="true" />
          </Button>
        </div>
      </div>
      {customOpen && (
        <div
          id="theme-quick-custom-highlight"
          className="min-w-0 rounded-lg border border-border bg-card p-3"
        >
          <Suspense
            fallback={
              <div className="flex h-64 items-center justify-center">
                <LoadingMark className="size-5" />
              </div>
            }
          >
            <HighlightColorPicker
              idPrefix="theme-quick-highlight"
              value={selectedColor}
              onPreview={setPreview}
              onApply={(value) => void select(value)}
              onCancel={closeCustom}
            />
          </Suspense>
        </div>
      )}
    </section>
  );
}

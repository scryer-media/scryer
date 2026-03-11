import { Button } from "@/components/ui/button";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Loader2 } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";

type AcquisitionSettings = {
  enabled: boolean;
  upgradeCooldownHours: number;
  sameTierMinDelta: number;
  crossTierMinDelta: number;
  forcedUpgradeDeltaBypass: number;
  pollIntervalSeconds: number;
  syncIntervalSeconds: number;
  batchSize: number;
};

type Props = {
  settings: AcquisitionSettings;
  setSettings: (s: AcquisitionSettings) => void;
  saving: boolean;
  loading: boolean;
  onSave: () => void;
};

export function SettingsAcquisitionSection({
  settings,
  setSettings,
  saving,
  loading,
  onSave,
}: Props) {
  const t = useTranslate();
  const update = (patch: Partial<AcquisitionSettings>) =>
    setSettings({ ...settings, ...patch });
  const parseIntegerInput = (raw: string) => {
    const nextValue = sanitizeDigits(raw);
    return nextValue === "" ? 0 : Number(nextValue);
  };

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("status.loading")}
      </div>
    );
  }

  return (
    <div className="space-y-6 text-sm">
      <div className="flex items-center gap-3">
        <Label>{t("settings.acq.enabled")}</Label>
        <button
          type="button"
          role="switch"
          aria-checked={settings.enabled}
          className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${settings.enabled ? "bg-primary" : "bg-muted"}`}
          onClick={() => update({ enabled: !settings.enabled })}
        >
          <span
            className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${settings.enabled ? "translate-x-5" : "translate-x-0"}`}
          />
        </button>
      </div>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <div className="space-y-1">
          <Label>{t("settings.acq.cooldownHours")}</Label>
          <Input
            {...integerInputProps}
            value={settings.upgradeCooldownHours}
            onChange={(e) => update({ upgradeCooldownHours: parseIntegerInput(e.target.value) })}
          />
        </div>
        <div className="space-y-1">
          <Label>{t("settings.acq.sameTierDelta")}</Label>
          <Input
            {...integerInputProps}
            value={settings.sameTierMinDelta}
            onChange={(e) => update({ sameTierMinDelta: parseIntegerInput(e.target.value) })}
          />
        </div>
        <div className="space-y-1">
          <Label>{t("settings.acq.crossTierDelta")}</Label>
          <Input
            {...integerInputProps}
            value={settings.crossTierMinDelta}
            onChange={(e) => update({ crossTierMinDelta: parseIntegerInput(e.target.value) })}
          />
        </div>
        <div className="space-y-1">
          <Label>{t("settings.acq.forcedBypassDelta")}</Label>
          <Input
            {...integerInputProps}
            value={settings.forcedUpgradeDeltaBypass}
            onChange={(e) =>
              update({ forcedUpgradeDeltaBypass: parseIntegerInput(e.target.value) })
            }
          />
        </div>
        <div className="space-y-1">
          <Label>{t("settings.acq.pollInterval")}</Label>
          <Input
            {...integerInputProps}
            value={settings.pollIntervalSeconds}
            onChange={(e) => update({ pollIntervalSeconds: parseIntegerInput(e.target.value) })}
          />
        </div>
        <div className="space-y-1">
          <Label>{t("settings.acq.syncInterval")}</Label>
          <Input
            {...integerInputProps}
            value={settings.syncIntervalSeconds}
            onChange={(e) => update({ syncIntervalSeconds: parseIntegerInput(e.target.value) })}
          />
        </div>
        <div className="space-y-1">
          <Label>{t("settings.acq.batchSize")}</Label>
          <Input
            {...integerInputProps}
            value={settings.batchSize}
            onChange={(e) => update({ batchSize: parseIntegerInput(e.target.value) })}
          />
        </div>
      </div>

      <Button onClick={onSave} disabled={saving}>
        {saving ? (
          <>
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            {t("settings.saving")}
          </>
        ) : (
          t("settings.save")
        )}
      </Button>
    </div>
  );
}

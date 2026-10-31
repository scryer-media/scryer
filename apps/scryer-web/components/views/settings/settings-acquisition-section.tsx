import { useEffect, useState } from "react";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useTranslate } from "@/lib/context/translate-context";
import { LoadingMark } from "@/components/common/loading-mark";

export type AcquisitionSettings = {
  enabled: boolean;
  sameTierMinDelta: number;
  pollIntervalSeconds: number;
  walkIntervalSeconds: number;
  longTailBackfillMaxScopesPerCycle: number;
  longTailReconvergeDays: number;
};

type Props = {
  settings: AcquisitionSettings | null;
  loading: boolean;
  saving: boolean;
  canManage: boolean;
  onSave: (next: AcquisitionSettings) => Promise<void>;
};

function NumberField({
  id,
  label,
  help,
  value,
  disabled,
  onChange,
}: {
  id: string;
  label: string;
  help: string;
  value: number;
  disabled: boolean;
  onChange: (value: number) => void;
}) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type="number"
        value={Number.isFinite(value) ? value : 0}
        disabled={disabled}
        onChange={(event) => {
          const next = Number.parseInt(event.target.value, 10);
          onChange(Number.isNaN(next) ? 0 : next);
        }}
        className="max-w-[220px]"
      />
      <p className="max-w-[560px] text-xs text-muted-foreground">{help}</p>
    </div>
  );
}

// Convergence-era acquisition settings. RSS is the steady-state
// acquisition path; active search converges each scope once per indexer and the
// backfill cursor is paced and finite — these knobs bound that work.
export function SettingsAcquisitionSection({
  settings,
  loading,
  saving,
  canManage,
  onSave,
}: Props) {
  const t = useTranslate();
  const [draft, setDraft] = useState<AcquisitionSettings | null>(settings);

  useEffect(() => {
    setDraft(settings);
  }, [settings]);

  if (loading || !draft) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <LoadingMark className="h-4 w-4" />
        {t("label.loading")}
      </div>
    );
  }

  const disabled = !canManage || saving;
  const update = (patch: Partial<AcquisitionSettings>) =>
    setDraft((current) => (current ? { ...current, ...patch } : current));

  return (
    <div id="settings-acquisition-section" className="max-w-[760px] space-y-6">
      <p className="text-sm text-muted-foreground">
        {t("settings.acquisitionIntro")}
      </p>

      <div className="flex flex-col gap-3 border-b border-border pb-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="space-y-1">
          <Label htmlFor="settings-acquisition-enabled-toggle">
            {t("settings.acquisitionEnabled")}
          </Label>
          <p className="max-w-[560px] text-xs text-muted-foreground">
            {t("settings.acquisitionEnabledHelp")}
          </p>
        </div>
        <SettingsToggleSwitch
          id="settings-acquisition-enabled-toggle"
          checked={draft.enabled}
          disabled={disabled}
          onChange={(enabled) => update({ enabled })}
        />
      </div>

      <div className="space-y-4">
        <h2 className="text-sm font-semibold text-foreground">
          {t("settings.acquisitionThresholds")}
        </h2>
        <NumberField
          id="settings-acquisition-same-tier-delta"
          label={t("settings.acquisitionSameTierMinDelta")}
          help={t("settings.acquisitionSameTierMinDeltaHelp")}
          value={draft.sameTierMinDelta}
          disabled={disabled}
          onChange={(sameTierMinDelta) => update({ sameTierMinDelta })}
        />
      </div>

      <div className="space-y-4">
        <h2 className="text-sm font-semibold text-foreground">
          {t("settings.acquisitionConvergence")}
        </h2>
        <NumberField
          id="settings-acquisition-poll-interval"
          label={t("settings.acquisitionPollIntervalSeconds")}
          help={t("settings.acquisitionPollIntervalSecondsHelp")}
          value={draft.pollIntervalSeconds}
          disabled={disabled}
          onChange={(pollIntervalSeconds) => update({ pollIntervalSeconds })}
        />
        <NumberField
          id="settings-acquisition-walk-interval"
          label={t("settings.acquisitionWalkIntervalSeconds")}
          help={t("settings.acquisitionWalkIntervalSecondsHelp")}
          value={draft.walkIntervalSeconds}
          disabled={disabled}
          onChange={(walkIntervalSeconds) => update({ walkIntervalSeconds })}
        />
        <NumberField
          id="settings-acquisition-max-scopes"
          label={t("settings.acquisitionMaxScopesPerCycle")}
          help={t("settings.acquisitionMaxScopesPerCycleHelp")}
          value={draft.longTailBackfillMaxScopesPerCycle}
          disabled={disabled}
          onChange={(longTailBackfillMaxScopesPerCycle) =>
            update({ longTailBackfillMaxScopesPerCycle })
          }
        />
        <NumberField
          id="settings-acquisition-reconverge-days"
          label={t("settings.acquisitionReconvergeDays")}
          help={t("settings.acquisitionReconvergeDaysHelp")}
          value={draft.longTailReconvergeDays}
          disabled={disabled}
          onChange={(longTailReconvergeDays) =>
            update({ longTailReconvergeDays })
          }
        />
      </div>

      <div>
        <Button
          id="settings-acquisition-save"
          disabled={disabled}
          onClick={() => void onSave(draft)}
        >
          {saving ? <LoadingMark className="mr-1 h-4 w-4" /> : null}
          {t("label.save")}
        </Button>
      </div>
    </div>
  );
}

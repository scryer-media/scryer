import { useState, type CSSProperties } from "react";
import { useMutation, useQuery } from "urql";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useTranslate } from "@/lib/context/translate-context";
import { fieldConditionHolds } from "@/lib/utils/provider-config-fields";
import type { FieldCondition } from "@/lib/types/indexers";

const settingsQuery = `query InstalledPluginSettings($pluginId: ID!) {
  installedPluginSettings(pluginId: $pluginId)
}`;
const settingsMutation = `mutation UpdateInstalledPluginSettings($pluginId: ID!, $changes: JSON!) {
  updateInstalledPluginSettings(pluginId: $pluginId, changes: $changes)
}`;

type SettingCondition = { key: string; op: string; values?: string[] };
type PluginSetting = {
  definition: {
    key: string;
    label: string;
    field_type: string;
    required: boolean;
    default_value?: string;
    visible_when?: SettingCondition;
    required_when?: SettingCondition;
    help_text?: string;
    options?: { value: string; label: string }[];
  };
  sensitive: boolean;
  isSet: boolean;
  hasDefault: boolean;
  visible: boolean;
  required: boolean;
  visibleWhenReset: boolean;
  requiredWhenReset: boolean;
  value: string | null;
};
type PluginSettings = { pluginId: string; fields: PluginSetting[] };

export function PluginSettingsPanel({ pluginId }: { pluginId: string }) {
  const t = useTranslate();
  const [result, refresh] = useQuery<{ installedPluginSettings: PluginSettings }>({
    query: settingsQuery, variables: { pluginId }, requestPolicy: "network-only",
  });
  const [saving, save] = useMutation(settingsMutation);
  const [changes, setChanges] = useState<Record<string, string | null>>({});
  const [revealed, setRevealed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fields = result.data?.installedPluginSettings.fields ?? [];
  if (result.error) return <div role="alert" className="mt-2 text-sm text-destructive">
    {t("settings.pluginSettingsLoadFailed")}
    <Button type="button" variant="ghost" onClick={() => refresh({ requestPolicy: "network-only" })}>{t("label.retry")}</Button>
  </div>;
  if (result.fetching && !result.data) return <p className="mt-2 text-xs text-muted-foreground">{t("settings.pluginSettingsLoading")}</p>;
  if (fields.length === 0) return null;

  const update = (key: string, value: string | null) => {
    setChanges((previous) => ({ ...previous, [key]: value }));
  };
  const submit = async () => {
    setError(null);
    const response = await save({ pluginId, changes });
    if (response.error) {
      setError(t("settings.pluginSettingsSaveFailed"));
      return;
    }
    setChanges({});
    setRevealed(false);
    refresh({ requestPolicy: "network-only" });
  };

  return <details className="mt-3 whitespace-normal" onToggle={(event) => {
    if (!event.currentTarget.open) { setChanges({}); setRevealed(false); setError(null); }
  }}>
    <summary className="cursor-pointer text-sm">{t("settings.pluginSettings")}</summary>
    <div className="mt-3 space-y-4" data-ui="plugin-settings-panel">
      {fields.map(({ definition, sensitive, isSet, hasDefault, value, visible, required, visibleWhenReset, requiredWhenReset }) => {
        const condition = (rule: SettingCondition | undefined, saved: boolean, reset: boolean) => {
          if (!rule || !Object.hasOwn(changes, rule.key)) return saved;
          const referenced = fields.find((field) => field.definition.key === rule.key);
          // The server evaluates a reset without exposing the sensitive default.
          if (changes[rule.key] === null && referenced?.sensitive && referenced.hasDefault) return reset;
          const op = ({ eq: "EQ", ne: "NE", in: "IN", not_in: "NOT_IN", non_empty: "NON_EMPTY" } as const)[rule.op as "eq" | "ne" | "in" | "not_in" | "non_empty"];
          if (!op) return false;
          const values = { [rule.key]: changes[rule.key] ?? referenced?.definition.default_value ?? "" };
          return fieldConditionHolds({ ...rule, op, values: rule.values ?? [] } satisfies FieldCondition, values);
        };
        const shown = condition(definition.visible_when, visible, visibleWhenReset);
        if (!shown) return null;
        const requiredNow = definition.required || condition(definition.required_when, required, requiredWhenReset);
        const id = `plugin-setting-${pluginId}-${definition.key}`;
        const edited = Object.hasOwn(changes, definition.key);
        const current = edited ? changes[definition.key] ?? "" : value ?? "";
        const label = definition.key === "additional_passwords"
          ? t("settings.additionalArchivePasswords") : definition.label;
        return <div key={definition.key} className="space-y-2">
          <Label htmlFor={id}>{label}{requiredNow ? " *" : ""}</Label>
          {definition.field_type === "multiline" ? <textarea
            id={id} value={current} disabled={saving.fetching} rows={5}
            autoComplete="off" spellCheck={false}
            className="w-full rounded-md border bg-background p-2 text-sm"
            style={sensitive && !revealed ? { WebkitTextSecurity: "disc" } as CSSProperties : undefined}
            onChange={(event) => update(definition.key, event.target.value)}
          /> : definition.options?.length ? <select id={id} value={current}
            disabled={saving.fetching} onChange={(event) => update(definition.key, event.target.value)}>
            <option value="" />
            {definition.options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
          </select> : definition.field_type === "bool" ? <input id={id} type="checkbox"
            checked={current === "true"} disabled={saving.fetching}
            onChange={(event) => update(definition.key, String(event.target.checked))} /> : <Input
            id={id} value={current} disabled={saving.fetching}
            type={sensitive && !revealed ? "password" : definition.field_type === "number" ? "number" : "text"}
            autoComplete="off" onChange={(event) => update(definition.key, event.target.value)} />}
          {sensitive && isSet && !edited && <p className="text-xs text-muted-foreground">{t("settings.pluginSecretSaved")}</p>}
          <p className="text-xs text-muted-foreground">{definition.key === "additional_passwords"
            ? t("settings.additionalArchivePasswordsHelp") : definition.help_text}</p>
          {(!requiredNow || hasDefault) && <Button type="button" variant="ghost" disabled={saving.fetching}
            onClick={() => update(definition.key, null)}>{t("label.clear")}</Button>}
        </div>;
      })}
      {fields.some((field) => field.sensitive) && <Button type="button" variant="ghost"
        onClick={() => setRevealed((value) => !value)}>{t(revealed ? "settings.hideEnteredPasswords" : "settings.showEnteredPasswords")}</Button>}
      <Button type="button" disabled={saving.fetching || Object.keys(changes).length === 0}
        onClick={() => { void submit(); }}>{t("label.save")}</Button>
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    </div>
  </details>;
}

import * as React from "react";
import { KeyRound } from "lucide-react";

import { LoadingMark } from "@/components/common/loading-mark";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useTranslate } from "@/lib/context/translate-context";
import type { ListProviderSettingChange, ListProviderSettingField } from "@/lib/types/lists";
import {
  type ListProviderSettingEdit,
  listProviderSettingChanges,
  listProviderSettingsMissing,
} from "@/lib/utils/lists";

type ProviderSettingsCardProps = {
  providerType: string;
  fields: ListProviderSettingField[];
  onSave: (provider: string, changes: ListProviderSettingChange[]) => Promise<boolean>;
};

function fieldId(providerType: string, key: string): string {
  return `lists-provider-setting-${providerType}-${key}`;
}

/**
 * A provider's server-wide settings, such as an API key every list from that
 * provider shares. Secrets are write-only: the form shows whether one is
 * stored and lets a manager replace or clear it.
 */
export function ProviderSettingsCard({ providerType, fields, onSave }: ProviderSettingsCardProps) {
  const t = useTranslate();
  const [edits, setEdits] = React.useState<Record<string, ListProviderSettingEdit>>({});
  const [saving, setSaving] = React.useState(false);
  const changes = React.useMemo(() => listProviderSettingChanges(fields, edits), [edits, fields]);
  const missing = React.useMemo(() => new Set(listProviderSettingsMissing(fields, edits)), [edits, fields]);

  const edit = (key: string, next: ListProviderSettingEdit) => setEdits((current) => ({ ...current, [key]: next }));

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (changes.length === 0) return;
    setSaving(true);
    try {
      if (await onSave(providerType, changes)) setEdits({});
    } finally {
      setSaving(false);
    }
  };

  return (
    <form
      id={`lists-provider-settings-${providerType}`}
      onSubmit={(event) => void submit(event)}
      className="space-y-3 rounded-[12px] border border-[var(--scry-border3)] bg-[var(--scry-surf)] p-4"
    >
      <div className="flex items-start gap-2">
        <KeyRound className="mt-0.5 h-4 w-4 flex-none text-[var(--scry-muted)]" />
        <div className="min-w-0">
          <h4 className="text-[13px] font-semibold text-[var(--scry-ink2)]">{t("lists.providerSettings.heading")}</h4>
          <p className="text-[12px] text-[var(--scry-muted)]">{t("lists.providerSettings.copy")}</p>
        </div>
      </div>
      {fields.map((field) => {
        const id = fieldId(providerType, field.key);
        const current = edits[field.key];
        const value = current?.value ?? (field.secret ? "" : (field.value ?? ""));
        const cleared = current?.clear ?? false;
        return (
          <div key={field.key} className="space-y-1.5">
            <div className="flex flex-wrap items-center gap-2">
              <label htmlFor={id} className="text-[12.5px] font-semibold text-[var(--scry-ink2)]">
                {field.label}
              </label>
              {field.required ? (
                <Badge tone={missing.has(field.key) ? "warning" : "outline"} className="px-1.5 py-0 text-[10.5px]">
                  {t("lists.providerSettings.required")}
                </Badge>
              ) : null}
              {field.secret ? (
                <Badge tone={field.isSet && !cleared ? "positive" : "neutral"} className="px-1.5 py-0 text-[10.5px]">
                  {field.isSet && !cleared ? t("lists.providerSettings.secretStored") : t("lists.providerSettings.secretEmpty")}
                </Badge>
              ) : null}
            </div>
            {field.type === "BOOL" ? (
              <Switch
                id={id}
                checked={value === "true"}
                onCheckedChange={(checked) => edit(field.key, { value: checked ? "true" : "false", clear: false })}
              />
            ) : field.type === "MULTILINE" && !field.secret ? (
              <Textarea
                id={id}
                rows={3}
                value={value}
                onChange={(event) => edit(field.key, { value: event.target.value, clear: false })}
              />
            ) : (
              <div className="flex flex-col gap-2 sm:flex-row">
                <Input
                  id={id}
                  className="min-w-0 flex-1"
                  type={field.secret || field.type === "PASSWORD" ? "password" : "text"}
                  inputMode={field.type === "NUMBER" ? "decimal" : undefined}
                  autoComplete={field.secret ? "new-password" : "off"}
                  placeholder={
                    field.secret && field.isSet && !cleared ? t("lists.providerSettings.secretPlaceholder") : undefined
                  }
                  value={value}
                  onChange={(event) => edit(field.key, { value: event.target.value, clear: false })}
                />
                {field.secret && field.isSet ? (
                  <Button
                    id={`${id}-clear`}
                    type="button"
                    variant="outline"
                    size="sm"
                    aria-pressed={cleared}
                    onClick={() => edit(field.key, { value: "", clear: !cleared })}
                  >
                    {cleared ? t("lists.providerSettings.keepSecret") : t("lists.providerSettings.clearSecret")}
                  </Button>
                ) : null}
              </div>
            )}
            {field.helpText ? <p className="text-[11.5px] text-[var(--scry-muted)]">{field.helpText}</p> : null}
          </div>
        );
      })}
      <div className="flex justify-end">
        <Button id={`lists-provider-settings-${providerType}-save`} type="submit" size="sm" disabled={saving || changes.length === 0}>
          {saving ? <LoadingMark className="h-4 w-4" /> : null}
          {t("lists.providerSettings.save")}
        </Button>
      </div>
    </form>
  );
}

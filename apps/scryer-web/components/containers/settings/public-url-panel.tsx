import * as React from "react";
import { useClient } from "urql";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useTranslate } from "@/lib/context/translate-context";
import { useAuth } from "@/lib/hooks/use-auth";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";
import { tlsSettingsQuery } from "@/lib/graphql/queries";
import { updateServiceSettingsMutation } from "@/lib/graphql/mutations";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import type { ConfigValueSource, ServiceSettings } from "@/lib/types/settings";

const SOURCE_KEYS: Record<ConfigValueSource, string> = {
  environment: "settings.publicUrlSourceEnvironment",
  settings: "settings.publicUrlSourceSettings",
  default: "settings.publicUrlSourceDefault",
};

const PASSKEY_SOURCE_KEYS: Record<ServiceSettings["passkeyRpSource"], string> = {
  environment: "settings.publicUrlSourceEnvironment",
  public_url: "settings.publicUrlSourcePublicUrl",
  none: "settings.publicUrlSourceDefault",
};

function hostnameOf(value: string): string | null {
  try {
    return new URL(value).hostname.toLowerCase();
  } catch {
    return null;
  }
}

// Whether saving `next` (null clears) would change the passkey relying party
// that existing passkeys are bound to.
function changesPasskeyRelyingParty(settings: ServiceSettings, next: string | null): boolean {
  if (!settings.passkeysRegistered || settings.passkeyRpSource !== "public_url") return false;
  if (next == null) return true;
  return hostnameOf(next) !== settings.passkeyRpId;
}

export function PublicUrlPanel() {
  const client = useClient();
  const t = useTranslate();
  const { user } = useAuth();
  const allowed = user != null && hasAppPermission(user, APP_PERMISSIONS.manageSystemSettings);
  const [settings, setSettings] = React.useState<ServiceSettings | null>(null);
  const [draft, setDraft] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [saved, setSaved] = React.useState(false);
  const [pendingReset, setPendingReset] = React.useState<boolean | null>(null);
  const inputId = React.useId();

  React.useEffect(() => {
    if (!allowed) return;
    let cancelled = false;
    void (async () => {
      try {
        const result = await client.query<{ serviceSettings: ServiceSettings }>(tlsSettingsQuery, {}, { requestPolicy: "network-only" }).toPromise();
        if (result.error) throw result.error;
        if (!result.data) throw new Error(t("settings.publicUrlLoadError"));
        if (!cancelled) {
          setSettings(result.data.serviceSettings);
          setDraft(result.data.serviceSettings.publicUrlSaved ?? "");
        }
      } catch (cause) {
        if (!cancelled) setError(userFacingGraphQlErrorMessage(cause, t("settings.publicUrlLoadError")));
      }
    })();
    return () => { cancelled = true; };
  }, [allowed, client, t]);

  function requestSave(reset: boolean) {
    if (busy || !settings) return;
    const next = reset || draft.trim() === "" ? null : draft.trim();
    if (changesPasskeyRelyingParty(settings, next)) {
      setPendingReset(reset);
      return;
    }
    void save(reset);
  }

  async function save(reset: boolean) {
    if (busy || !settings) return;
    setBusy(true);
    setError(null);
    setSaved(false);
    const clearing = reset || draft.trim() === "";
    try {
      const result = await client.mutation<{ updateServiceSettings: ServiceSettings }>(updateServiceSettingsMutation, {
        input: clearing ? { resetPublicUrl: true } : { publicUrl: draft.trim() },
      }).toPromise();
      if (result.error) throw result.error;
      if (!result.data) throw new Error(t("settings.publicUrlSaveError"));
      setSettings(result.data.updateServiceSettings);
      setDraft(result.data.updateServiceSettings.publicUrlSaved ?? "");
      setSaved(true);
    } catch (cause) {
      setError(userFacingGraphQlErrorMessage(cause, t("settings.publicUrlSaveError")));
    } finally {
      setBusy(false);
    }
  }

  if (!allowed) return null;
  const editable = settings?.publicUrlEditable ?? false;
  const none = t("settings.publicUrlNotSet");
  const rows: Array<[string, string, string]> = settings ? [
    [t("settings.publicUrlEffective"), settings.publicUrl ?? none, t(SOURCE_KEYS[settings.publicUrlSource])],
    [t("settings.publicUrlBasePath"), settings.basePath || "/", t(SOURCE_KEYS[settings.basePathSource])],
    [t("settings.publicUrlBindAddress"), settings.bindAddress, t(SOURCE_KEYS[settings.bindSource])],
    [t("settings.publicUrlTrustedProxies"), settings.trustedProxyIps.length > 0 ? settings.trustedProxyIps.join(", ") : none,
      t(settings.trustedProxySource === "settings" ? SOURCE_KEYS.settings : SOURCE_KEYS.environment)],
    [t("settings.publicUrlPasskeyRpId"), settings.passkeyRpId ?? none, t(PASSKEY_SOURCE_KEYS[settings.passkeyRpSource])],
    [t("settings.publicUrlPasskeyRpOrigin"), settings.passkeyRpOrigin ?? none, t(PASSKEY_SOURCE_KEYS[settings.passkeyRpSource])],
  ] : [];

  return (
    <section className="mt-6 space-y-4 rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-5" aria-busy={busy}>
      <h3 className="font-semibold">{t("settings.publicUrlTitle")}</h3>
      <p className="text-sm text-[var(--scry-muted)]">{t("settings.publicUrlHelp")}</p>
      {!settings && !error ? <p role="status">{t("label.loading")}</p> : null}
      <label className="block text-sm" htmlFor={inputId}>{t("settings.publicUrlLabel")}</label>
      <input id={inputId} type="url" inputMode="url" autoComplete="off" spellCheck={false}
        placeholder="https://scryer.example.com"
        value={editable ? draft : settings?.publicUrlSaved ?? ""} disabled={busy || !settings || !editable}
        onChange={(event) => { setDraft(event.target.value); setSaved(false); }}
        className="w-full rounded-md border border-[var(--scry-border)] bg-[var(--scry-bg)] p-3 font-mono text-sm disabled:opacity-50" />
      {settings && !editable ? <p className="text-sm text-[var(--scry-muted)]">{t("settings.publicUrlEnvironmentLocked")}</p> : null}
      {settings?.publicUrlError ? <p role="alert" className="text-sm text-red-400">{t("settings.publicUrlInvalid", { error: settings.publicUrlError })}</p> : null}
      <p className="text-sm text-[var(--scry-muted)]">{t("settings.publicUrlPasskeyRestart")}</p>
      <div className="flex flex-wrap gap-3">
        <Button disabled={busy || !settings || !editable} onClick={() => requestSave(false)}>{t(busy ? "label.saving" : "label.save")}</Button>
        <Button variant="outline" disabled={busy || !settings || !editable || settings.publicUrlSaved == null} onClick={() => requestSave(true)}>{t("settings.publicUrlClear")}</Button>
      </div>
      {error ? <p role="alert" className="text-sm text-red-400">{error}</p> : null}
      {saved ? <p role="status" className="text-sm text-green-400">{t("settings.publicUrlSaved")}</p> : null}
      {settings ? (
        <div className="overflow-x-auto">
          <h4 className="mb-2 text-sm font-semibold">{t("settings.publicUrlEffectiveValues")}</h4>
          <table className="w-full text-left text-sm">
            <thead className="text-[var(--scry-muted)]">
              <tr>
                <th className="py-1 pr-4 font-normal">{t("settings.publicUrlColumnSetting")}</th>
                <th className="py-1 pr-4 font-normal">{t("settings.publicUrlColumnValue")}</th>
                <th className="py-1 font-normal">{t("settings.publicUrlColumnSource")}</th>
              </tr>
            </thead>
            <tbody>
              {rows.map(([label, value, source]) => (
                <tr key={label} className="border-t border-[var(--scry-border)]">
                  <td className="py-1 pr-4">{label}</td>
                  <td className="py-1 pr-4 font-mono break-all">{value}</td>
                  <td className="py-1">{source}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
      <ConfirmDialog
        open={pendingReset != null}
        title={t("settings.publicUrlPasskeyWarningTitle")}
        description={t("settings.publicUrlPasskeyWarning", { rpId: settings?.passkeyRpId ?? "" })}
        confirmLabel={t("settings.publicUrlPasskeyWarningConfirm")}
        cancelLabel={t("label.cancel")}
        isBusy={busy}
        onConfirm={() => {
          const reset = pendingReset ?? false;
          setPendingReset(null);
          void save(reset);
        }}
        onCancel={() => setPendingReset(null)}
      />
    </section>
  );
}

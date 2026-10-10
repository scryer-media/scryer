import * as React from "react";
import { useClient } from "urql";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { useTranslate } from "@/lib/context/translate-context";
import { useAuth } from "@/lib/hooks/use-auth";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";
import { publicUrlChangePreviewQuery, tlsSettingsQuery } from "@/lib/graphql/queries";
import { updateServiceSettingsMutation } from "@/lib/graphql/mutations";
import { graphQlErrorExtension, userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import type {
  ConfigValueSource,
  PasskeyRelyingPartySource,
  PublicUrlChangePreview,
  PublicUrlErrorCode,
  ServiceSettings,
} from "@/lib/types/settings";

const SOURCE_KEYS: Record<ConfigValueSource, string> = {
  ENVIRONMENT: "settings.publicUrlSourceEnvironment",
  SETTINGS: "settings.publicUrlSourceSettings",
  DEFAULT: "settings.publicUrlSourceDefault",
};

const PASSKEY_SOURCE_KEYS: Record<PasskeyRelyingPartySource, string> = {
  ENVIRONMENT: "settings.publicUrlSourceEnvironment",
  PUBLIC_URL: "settings.publicUrlSourcePublicUrl",
  NONE: "settings.publicUrlSourceDefault",
};

const ERROR_KEYS: Record<PublicUrlErrorCode, string> = {
  INVALID_URL: "settings.publicUrlError.invalid_url",
  WILDCARD_HOST: "settings.publicUrlError.wildcard_host",
  CREDENTIALS: "settings.publicUrlError.credentials",
  QUERY_OR_FRAGMENT: "settings.publicUrlError.query_or_fragment",
  PATH_NOT_ALLOWED: "settings.publicUrlError.path_not_allowed",
  PATH_MISMATCH: "settings.publicUrlError.path_mismatch",
  ENVIRONMENT_LOCKED: "settings.publicUrlError.environment_locked",
  SAVE_AND_RESET: "settings.publicUrlError.save_and_reset",
  PASSKEY_ACKNOWLEDGEMENT_REQUIRED: "settings.publicUrlError.passkey_acknowledgement_required",
};

function isPublicUrlErrorCode(value: string | null): value is PublicUrlErrorCode {
  return value != null && Object.hasOwn(ERROR_KEYS, value);
}

type PendingSave = { reset: boolean; preview: PublicUrlChangePreview };

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
  const [pending, setPending] = React.useState<PendingSave | null>(null);
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

  function errorText(code: string | null, fallback: string): string {
    return isPublicUrlErrorCode(code) ? t(ERROR_KEYS[code]) : fallback;
  }

  // The server decides what the change does to passkeys, using the same rules
  // as the save and the next start; the save then enforces the same answer.
  async function requestSave(reset: boolean) {
    if (busy || !settings) return;
    const clearing = reset || draft.trim() === "";
    setBusy(true);
    setError(null);
    setSaved(false);
    let preview: PublicUrlChangePreview;
    try {
      const result = await client.query<{ publicUrlChangePreview: PublicUrlChangePreview }>(
        publicUrlChangePreviewQuery,
        clearing ? { reset: true } : { publicUrl: draft.trim() },
        { requestPolicy: "network-only" },
      ).toPromise();
      if (result.error) throw result.error;
      if (!result.data) throw new Error(t("settings.publicUrlSaveError"));
      preview = result.data.publicUrlChangePreview;
    } catch (cause) {
      setError(userFacingGraphQlErrorMessage(cause, t("settings.publicUrlSaveError")));
      setBusy(false);
      return;
    }
    setBusy(false);
    if (preview.errorCode != null || preview.error != null) {
      setError(errorText(preview.errorCode, preview.error ?? t("settings.publicUrlSaveError")));
      return;
    }
    if (preview.acknowledgementRequired) {
      setPending({ reset: clearing, preview });
      return;
    }
    void save(clearing, false);
  }

  async function save(clearing: boolean, acknowledgePasskeyImpact: boolean) {
    if (busy || !settings) return;
    setBusy(true);
    setError(null);
    setSaved(false);
    const input: Record<string, unknown> = clearing ? { resetPublicUrl: true } : { publicUrl: draft.trim() };
    if (acknowledgePasskeyImpact) input.acknowledgePasskeyImpact = true;
    try {
      const result = await client.mutation<{ updateServiceSettings: ServiceSettings }>(updateServiceSettingsMutation, {
        input,
      }).toPromise();
      if (result.error) throw result.error;
      if (!result.data) throw new Error(t("settings.publicUrlSaveError"));
      setSettings(result.data.updateServiceSettings);
      setDraft(result.data.updateServiceSettings.publicUrlSaved ?? "");
      setSaved(true);
    } catch (cause) {
      setError(errorText(
        graphQlErrorExtension(cause, "PUBLIC_URL_REJECTED", "reason"),
        userFacingGraphQlErrorMessage(cause, t("settings.publicUrlSaveError")),
      ));
    } finally {
      setBusy(false);
    }
  }

  function passkeyWarning(preview: PublicUrlChangePreview): string {
    const rpId = preview.currentPasskeyRpId ?? "";
    const parts = [
      preview.passkeyImpact === "DISABLED"
        ? t("settings.publicUrlPasskeyWarningDisabled", { rpId })
        : t("settings.publicUrlPasskeyWarningChanged", { rpId, nextRpId: preview.nextPasskeyRpId ?? "" }),
      preview.passkeyUserCount == null || preview.passkeyOnlyUserCount == null
        ? t("settings.publicUrlPasskeyWarningCountsUnknown")
        : t("settings.publicUrlPasskeyWarningCounts", {
          users: String(preview.passkeyUserCount),
          passkeyOnly: String(preview.passkeyOnlyUserCount),
        }),
      t("settings.publicUrlPasskeyWarningRecovery", {
        users: t("settings.users"),
        reset: t("settings.resetMfa"),
      }),
    ];
    return parts.join(" ");
  }

  if (!allowed) return null;
  const editable = settings?.publicUrlEditable ?? false;
  const none = t("settings.publicUrlNotSet");
  const rows: Array<[string, string, string]> = settings ? [
    [t("settings.publicUrlEffective"), settings.publicUrl ?? none, t(SOURCE_KEYS[settings.publicUrlSource])],
    [t("settings.publicUrlBasePath"), settings.basePath || "/", t(SOURCE_KEYS[settings.basePathSource])],
    [t("settings.publicUrlBindAddress"), settings.bindAddress, t(SOURCE_KEYS[settings.bindSource])],
    [t("settings.publicUrlTrustedProxies"), settings.trustedProxyIps.length > 0 ? settings.trustedProxyIps.join(", ") : none,
      t(settings.trustedProxySource === "settings" ? SOURCE_KEYS.SETTINGS : SOURCE_KEYS.ENVIRONMENT)],
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
        value={editable ? draft : settings?.publicUrl ?? ""} disabled={busy || !settings || !editable}
        onChange={(event) => { setDraft(event.target.value); setSaved(false); }}
        className="w-full rounded-md border border-[var(--scry-border)] bg-[var(--scry-bg)] p-3 font-mono text-sm disabled:opacity-50" />
      {settings && !editable ? <p className="text-sm text-[var(--scry-muted)]">{t("settings.publicUrlEnvironmentLocked")}</p> : null}
      {settings?.publicUrlError ? <p role="alert" className="text-sm text-red-400">{t("settings.publicUrlInvalid", { error: errorText(settings.publicUrlErrorCode, settings.publicUrlError) })}</p> : null}
      <p className="text-sm text-[var(--scry-muted)]">{t("settings.publicUrlPasskeyRestart")}</p>
      <div className="flex flex-wrap gap-3">
        <Button disabled={busy || !settings || !editable} onClick={() => void requestSave(false)}>{t(busy ? "label.saving" : "label.save")}</Button>
        <Button variant="outline" disabled={busy || !settings || !editable || settings.publicUrlSaved == null} onClick={() => void requestSave(true)}>{t("settings.publicUrlClear")}</Button>
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
        open={pending != null}
        title={t("settings.publicUrlPasskeyWarningTitle")}
        description={pending ? passkeyWarning(pending.preview) : ""}
        confirmLabel={t("settings.publicUrlPasskeyWarningConfirm")}
        cancelLabel={t("label.cancel")}
        isBusy={busy}
        onConfirm={() => {
          const clearing = pending?.reset ?? false;
          setPending(null);
          void save(clearing, true);
        }}
        onCancel={() => setPending(null)}
      />
    </section>
  );
}

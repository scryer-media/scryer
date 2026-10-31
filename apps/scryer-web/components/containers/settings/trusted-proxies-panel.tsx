import * as React from "react";
import { useClient } from "urql";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import { useAuth } from "@/lib/hooks/use-auth";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";
import { tlsSettingsQuery } from "@/lib/graphql/queries";
import { updateServiceSettingsMutation } from "@/lib/graphql/mutations";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import type { ServiceSettings } from "@/lib/types/settings";

export function TrustedProxiesPanel() {
  const client = useClient();
  const t = useTranslate();
  const { user } = useAuth();
  const allowed =
    user != null &&
    hasAppPermission(user, APP_PERMISSIONS.manageSystemSettings);
  const [settings, setSettings] = React.useState<ServiceSettings | null>(null);
  const [draft, setDraft] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [saved, setSaved] = React.useState(false);
  const inputId = React.useId();

  React.useEffect(() => {
    if (!allowed) return;
    let cancelled = false;
    void (async () => {
      try {
        const result = await client
          .query<{ serviceSettings: ServiceSettings }>(
            tlsSettingsQuery,
            {},
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (result.error) throw result.error;
        if (!result.data)
          throw new Error(t("settings.trustedProxiesLoadError"));
        if (!cancelled) {
          setSettings(result.data.serviceSettings);
          setDraft(result.data.serviceSettings.trustedProxyIps.join("\n"));
        }
      } catch (cause) {
        if (!cancelled)
          setError(
            userFacingGraphQlErrorMessage(
              cause,
              t("settings.trustedProxiesLoadError"),
              t,
            ),
          );
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [allowed, client, t]);

  async function save(reset: boolean) {
    if (busy || !settings) return;
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      const result = await client
        .mutation<{ updateServiceSettings: ServiceSettings }>(
          updateServiceSettingsMutation,
          {
            input: {
              ...(reset
                ? { resetTrustedProxyIps: true }
                : {
                    trustedProxyIps: draft
                      .split(/[\n,]/)
                      .map((value) => value.trim())
                      .filter(Boolean),
                  }),
            },
          },
        )
        .toPromise();
      if (result.error) throw result.error;
      if (!result.data) throw new Error(t("settings.trustedProxiesSaveError"));
      setSettings(result.data.updateServiceSettings);
      setDraft(result.data.updateServiceSettings.trustedProxyIps.join("\n"));
      setSaved(true);
    } catch (cause) {
      setError(
        userFacingGraphQlErrorMessage(
          cause,
          t("settings.trustedProxiesSaveError"),
          t,
        ),
      );
    } finally {
      setBusy(false);
    }
  }

  if (!allowed) return null;
  return (
    <section
      className="mt-6 space-y-4 rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-5"
      aria-busy={busy}
    >
      <h3 className="font-semibold">{t("settings.trustedProxiesTitle")}</h3>
      <p className="text-sm text-[var(--scry-muted)]">
        {t("settings.trustedProxiesHelp")}
      </p>
      {settings ? (
        <p className="text-sm">
          {t(
            settings.trustedProxySource === "settings"
              ? "settings.trustedProxiesSavedSource"
              : "settings.trustedProxiesEnvironmentSource",
          )}
        </p>
      ) : !error ? (
        <p role="status">{t("label.loading")}</p>
      ) : null}
      <label className="block text-sm" htmlFor={inputId}>
        {t("settings.trustedProxiesAddresses")}
      </label>
      <textarea
        id={inputId}
        rows={4}
        value={draft}
        disabled={busy || !settings}
        onChange={(event) => {
          setDraft(event.target.value);
          setSaved(false);
        }}
        className="w-full rounded-md border border-[var(--scry-border)] bg-[var(--scry-bg)] p-3 font-mono text-sm disabled:opacity-50"
      />
      <p className="text-sm text-[var(--scry-muted)]">
        {t("settings.trustedProxiesEmptyHelp")}
      </p>
      <div className="flex flex-wrap gap-3">
        <Button disabled={busy || !settings} onClick={() => void save(false)}>
          {t(busy ? "label.saving" : "label.save")}
        </Button>
        <Button
          variant="outline"
          disabled={busy || !settings || settings.trustedProxyOverride == null}
          onClick={() => void save(true)}
        >
          {t("settings.trustedProxiesReset")}
        </Button>
      </div>
      {error ? (
        <p role="alert" className="text-sm text-red-400">
          {error}
        </p>
      ) : null}
      {saved ? (
        <p role="status" className="text-sm text-green-400">
          {t("settings.trustedProxiesSaved")}
        </p>
      ) : null}
    </section>
  );
}

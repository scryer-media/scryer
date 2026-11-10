import * as React from "react";
import { useClient } from "urql";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { useTranslate } from "@/lib/context/translate-context";
import { useAuth } from "@/lib/hooks/use-auth";
import { useNetworkRestart } from "@/lib/hooks/use-network-restart";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";
import { tlsSettingsQuery } from "@/lib/graphql/queries";
import { updateServiceSettingsMutation } from "@/lib/graphql/mutations";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import {
  hostForReach,
  NETWORK_FALLBACK_KEYS,
  reachForHost,
  socketHost,
  type NetworkReach,
} from "@/lib/utils/network-bind";
import type { ConfigValueSource, ServiceSettings } from "@/lib/types/settings";

const SOURCE_KEYS: Record<ConfigValueSource, string> = {
  ENVIRONMENT: "settings.publicUrlSourceEnvironment",
  SETTINGS: "settings.publicUrlSourceSettings",
  DEFAULT: "settings.publicUrlSourceDefault",
};

/** The host the page should open on: the saved one, else what is in effect. */
function configuredHost(settings: ServiceSettings): string {
  return settings.bindSavedHost ?? socketHost(settings.bindPending);
}

/** Who can reach Scryer: the listening address, as two plain choices. */
export function NetworkBindPanel({ onSaved }: { onSaved?: () => void }) {
  const client = useClient();
  const t = useTranslate();
  const { user } = useAuth();
  const allowed =
    user != null &&
    hasAppPermission(user, APP_PERMISSIONS.manageSystemSettings);
  const [settings, setSettings] = React.useState<ServiceSettings | null>(null);
  const [reach, setReach] = React.useState<NetworkReach>("local");
  const [customHost, setCustomHost] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [saved, setSaved] = React.useState(false);
  const restart = useNetworkRestart();
  const customInputId = React.useId();

  const adopt = React.useCallback((next: ServiceSettings) => {
    setSettings(next);
    const host = configuredHost(next);
    const nextReach = reachForHost(host);
    setReach(nextReach);
    setCustomHost(nextReach === "custom" ? host : "");
  }, []);

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
        if (!result.data) throw new Error(t("settings.networkLoadError"));
        if (!cancelled) adopt(result.data.serviceSettings);
      } catch (cause) {
        if (!cancelled)
          setError(
            userFacingGraphQlErrorMessage(
              cause,
              t("settings.networkLoadError"),
              t,
            ),
          );
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [adopt, allowed, client, t]);

  async function save(input: Record<string, unknown>) {
    if (busy || !settings) return;
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      const result = await client
        .mutation<{ updateServiceSettings: ServiceSettings }>(
          updateServiceSettingsMutation,
          { input },
        )
        .toPromise();
      if (result.error) throw result.error;
      if (!result.data) throw new Error(t("settings.networkSaveError"));
      adopt(result.data.updateServiceSettings);
      setSaved(true);
      onSaved?.();
    } catch (cause) {
      setError(
        userFacingGraphQlErrorMessage(cause, t("settings.networkSaveError"), t),
      );
    } finally {
      setBusy(false);
    }
  }

  if (!allowed) return null;
  const editable = settings?.bindEditable ?? false;
  const host = hostForReach(reach, customHost);
  const restarting =
    restart.phase === "requesting" || restart.phase === "restarting";
  const fallbackKey = settings?.bindFallbackCode
    ? NETWORK_FALLBACK_KEYS[settings.bindFallbackCode]
    : undefined;
  const controlsDisabled = busy || !settings || !editable || restarting;

  return (
    <section
      className="space-y-4 rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-4"
      aria-busy={busy || restarting}
    >
      <div className="-mx-4 -mt-4 mb-4 flex flex-wrap items-center gap-2 border-b border-[var(--scry-line2)] px-4 py-3">
        <h4 className="text-sm font-semibold text-[var(--scry-ink)]">
          {t("settings.networkTitle")}
        </h4>
        {settings ? (
          <Badge tone="neutral">{t(SOURCE_KEYS[settings.bindSource])}</Badge>
        ) : null}
      </div>
      <p className="text-sm text-[var(--scry-muted)]">
        {t("settings.networkHelp")}
      </p>
      {!settings && !error ? <p role="status">{t("label.loading")}</p> : null}
      {settings?.bindFallbackCode || settings?.bindFallbackReason ? (
        <div
          role="alert"
          className="rounded-md border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-3 text-sm text-[var(--scry-warning-text)]"
        >
          <p className="font-medium">
            {t("settings.networkFallbackTitle", {
              address: settings.bindAddress,
            })}
          </p>
          <p>
            {fallbackKey
              ? t(fallbackKey, {
                  address: settings.bindFallbackAddress ?? "",
                })
              : (settings.bindFallbackReason ?? "")}
          </p>
        </div>
      ) : null}
      {settings && !editable ? (
        <p className="text-sm text-[var(--scry-muted)]">
          {t(
            settings.bindPinnedBy === "CONTAINER"
              ? "settings.networkPinnedContainer"
              : "settings.networkPinnedEnvironment",
          )}
        </p>
      ) : null}
      <RadioGroup
        value={reach}
        disabled={controlsDisabled}
        onValueChange={(value) => {
          setReach(value as NetworkReach);
          setSaved(false);
        }}
        aria-label={t("settings.networkTitle")}
      >
        {(["local", "network"] as const).map((choice) => (
          <label key={choice} className="flex items-start gap-3 text-sm">
            <RadioGroupItem value={choice} className="mt-0.5" />
            <span>
              <span className="block font-medium">
                {t(
                  choice === "local"
                    ? "initialAuth.networkLocal"
                    : "initialAuth.networkOther",
                )}
              </span>
              <span className="block text-[var(--scry-muted)]">
                {t(
                  choice === "local"
                    ? "initialAuth.networkLocalBody"
                    : "initialAuth.networkOtherBody",
                )}
              </span>
            </span>
          </label>
        ))}
        <details open={reach === "custom"} className="text-sm">
          <summary className="cursor-pointer text-[var(--scry-muted)]">
            {t("settings.networkAdvanced")}
          </summary>
          <div className="mt-3 space-y-2">
            <label className="flex items-start gap-3">
              <RadioGroupItem value="custom" className="mt-0.5" />
              <span className="block font-medium">
                {t("settings.networkCustom")}
              </span>
            </label>
            <label className="block" htmlFor={customInputId}>
              {t("settings.networkCustomLabel")}
            </label>
            <input
              id={customInputId}
              type="text"
              inputMode="decimal"
              autoComplete="off"
              spellCheck={false}
              placeholder="192.168.1.20"
              value={customHost}
              disabled={controlsDisabled}
              onChange={(event) => {
                setCustomHost(event.target.value);
                setReach("custom");
                setSaved(false);
              }}
              className="w-full rounded-md border border-[var(--scry-border)] bg-[var(--scry-bg)] p-3 font-mono text-sm disabled:opacity-50"
            />
            <p className="text-[var(--scry-muted)]">
              {t("settings.networkCustomHelp")}
            </p>
          </div>
        </details>
      </RadioGroup>
      <div className="flex flex-wrap gap-3">
        <Button
          disabled={controlsDisabled || host == null}
          onClick={() => {
            if (host != null) void save({ bindHost: host });
          }}
        >
          {t(busy ? "label.saving" : "label.save")}
        </Button>
        <Button
          variant="outline"
          disabled={controlsDisabled || settings?.bindSavedHost == null}
          onClick={() => void save({ resetBindHost: true })}
        >
          {t("settings.networkClear")}
        </Button>
      </div>
      {error ? (
        <p role="alert" className="text-sm text-red-400">
          {error}
        </p>
      ) : null}
      {saved && !settings?.bindRestartRequired ? (
        <p role="status" className="text-sm text-green-400">
          {t("settings.networkSaved")}
        </p>
      ) : null}
      {settings?.bindRestartRequired ? (
        <div className="space-y-3 rounded-md border border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] p-3 text-sm text-[var(--scry-info-text)]">
          <p role="status">
            {t("settings.networkRestartPending", {
              address: settings.bindPending,
            })}
          </p>
          {restart.phase === "restarting" ? (
            <p role="status">{t("settings.networkRestarting")}</p>
          ) : null}
          {restart.phase === "unreachable" ? (
            <p role="alert">
              {t("settings.networkRestartUnreachable", {
                address: settings.bindPending,
              })}
            </p>
          ) : null}
          {restart.error ? <p role="alert">{restart.error}</p> : null}
          <Button disabled={restarting} onClick={() => void restart.restart()}>
            {t(
              restarting
                ? "settings.networkRestarting"
                : "settings.networkRestartNow",
            )}
          </Button>
        </div>
      ) : null}
    </section>
  );
}

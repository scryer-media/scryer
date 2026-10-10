import * as React from "react";
import { useClient } from "urql";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import { listProviderAppsQuery, updateListProviderAppMutation } from "@/lib/graphql/list-accounts";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import type { ListProviderApp } from "@/lib/types/lists";

export function ProviderAppsPanel() {
  const client = useClient();
  const t = useTranslate();
  const [apps, setApps] = React.useState<ListProviderApp[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [retry, setRetry] = React.useState(0);
  React.useEffect(() => {
    let cancelled = false;
    void client.query(listProviderAppsQuery, {}, { requestPolicy: "network-only" }).toPromise().then((result) => {
      if (cancelled) return;
      setError(result.error ? userFacingGraphQlErrorMessage(result.error, t("status.failedToLoad")) : null);
      if (!result.error) setApps(result.data?.listProviderApps ?? []);
    });
    return () => { cancelled = true; };
  }, [client, retry, t]);
  return (
    <section id="lists-provider-apps" className="space-y-4">
      <div className="flex items-center gap-2.5">
        <h2 className="text-lg font-semibold">{t("lists.providerApps.heading")}</h2>
        <Badge tone="outline">{t("label.advanced")}</Badge>
      </div>
      <p className="text-sm text-muted-foreground">{t("lists.providerApps.copy")}</p>
      <p className="text-sm text-muted-foreground">{t("lists.providerApps.simklDefault")}</p>
      {error ? <div role="alert"><p>{error}</p><Button onClick={() => setRetry((value) => value + 1)}>{t("lists.action.retry")}</Button></div> : null}
      {apps === null && !error ? <p role="status">{t("label.loading")}</p> : null}
      {apps?.filter((app) => ["trakt", "anilist", "mal"].includes(app.provider)).map((app) => (
        <ProviderAppForm key={app.provider} app={app} onSaved={(saved) => setApps((current) => current?.map((entry) => entry.provider === saved.provider ? saved : entry) ?? null)} />
      ))}
    </section>
  );
}

function ProviderAppForm({ app, onSaved }: { app: ListProviderApp; onSaved: (app: ListProviderApp) => void }) {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const [clientId, setClientId] = React.useState(app.clientId ?? "");
  const [clientSecret, setClientSecret] = React.useState("");
  const callbackUri = `${window.location.origin}${getRuntimeBasePath().replace(/\/$/, "")}/lists/oauth/callback`;
  const [redirectUri, setRedirectUri] = React.useState(app.redirectUri ?? callbackUri);
  const [enabled, setEnabled] = React.useState(app.enabled);
  const [saving, setSaving] = React.useState(false);
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setSaving(true);
    try {
      const result = await client.mutation(updateListProviderAppMutation, { provider: app.provider, clientId: clientId.trim() || null, clientSecret: clientSecret || null, redirectUri: redirectUri.trim() || null, enabled }).toPromise();
      if (result.error) throw result.error;
      onSaved(result.data.updateListProviderApp);
      setClientSecret("");
      setGlobalStatus(t("lists.providerSettings.saved"));
    } catch (reason) { setGlobalStatus(userFacingGraphQlErrorMessage(reason, t("status.failedToUpdate"))); }
    finally { setSaving(false); }
  };
  return (
    <form onSubmit={(event) => void submit(event)} className="space-y-3 rounded-xl border p-4">
      <h3 className="font-semibold">{app.provider === "mal" ? "MyAnimeList" : app.provider === "anilist" ? "AniList" : "Trakt"}</h3>
      <div className="flex items-center gap-2"><Switch id={`provider-app-${app.provider}-enabled`} checked={enabled} onCheckedChange={setEnabled} /><label htmlFor={`provider-app-${app.provider}-enabled`}>{t("lists.providerApps.useOwn")}</label></div>
      <label className="block space-y-1"><span>{t("lists.providerApps.clientId")}</span><Input value={clientId} autoComplete="off" onChange={(event) => setClientId(event.target.value)} /></label>
      <label className="block space-y-1"><span>{t("lists.providerApps.clientSecret")}</span><Input type="password" value={clientSecret} autoComplete="new-password" placeholder={app.clientSecretSet ? t("lists.providerSettings.secretPlaceholder") : undefined} onChange={(event) => setClientSecret(event.target.value)} />{app.provider === "trakt" ? <span className="block text-sm text-muted-foreground">{t("lists.providerApps.traktSecretHint")}</span> : null}</label>
      <label className="block space-y-1"><span>{t("lists.providerApps.redirectUri")}</span><Input value={redirectUri} inputMode="url" autoComplete="off" onChange={(event) => setRedirectUri(event.target.value)} /></label>
      <p className="text-sm text-muted-foreground">{t("lists.providerApps.redirectHelp")}</p>
      <code className="block select-all break-all text-sm">{callbackUri}</code>
      <Button type="submit" disabled={saving || (enabled && (!clientId.trim() || !redirectUri.trim()))}>{t("label.save")}</Button>
    </form>
  );
}

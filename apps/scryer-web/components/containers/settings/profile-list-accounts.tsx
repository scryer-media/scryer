import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router";
import { useClient } from "urql";
import { LoadingMark } from "@/components/common/loading-mark";
import { Button } from "@/components/ui/button";
import { SettingsProfileCard } from "@/components/views/settings/settings-profile-section";
import { ListProviderSetup } from "@/components/views/lists/list-provider-setup";
import { PersonalAccountConnections } from "@/components/views/lists/personal-accounts";
import { FilteredPluginList } from "@/components/views/settings/filtered-plugin-list";
import { useTranslate } from "@/lib/context/translate-context";
import { myListAccountsQuery } from "@/lib/graphql/list-accounts";
import { listProvidersQuery } from "@/lib/graphql/queries";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { useListAccountLink } from "@/lib/hooks/use-list-account-link";
import { useSessionUser } from "@/lib/hooks/use-auth";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";
import type { ListAccount, ListProviderManifest } from "@/lib/types/lists";
import { buildListsPath } from "@/lib/utils/routing";

export function ProfileListAccounts() {
  const client = useClient();
  const t = useTranslate();
  const user = useSessionUser();
  const canManagePlugins = hasAppPermission(user, APP_PERMISSIONS.manageSystemSettings);
  const [providers, setProviders] = useState<ListProviderManifest[]>([]);
  const [accounts, setAccounts] = useState<ListAccount[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const loadAccounts = useCallback(async (isCurrent: () => boolean) => {
    const result = await client.query(myListAccountsQuery, {}, { requestPolicy: "network-only" }).toPromise();
    if (!isCurrent()) return;
    if (result.error) throw result.error;
    setAccounts((result.data?.myListAccounts ?? []) as ListAccount[]);
  }, [client]);
  const accountLink = useListAccountLink(loadAccounts);
  const refreshProviders = useCallback(async () => {
    const [result] = await Promise.all([
      client.query(listProvidersQuery, {}, { requestPolicy: "network-only" }).toPromise(),
      loadAccounts(() => true),
    ]);
    if (result.error) throw result.error;
    setProviders((result.data?.listProviders ?? []) as ListProviderManifest[]);
    setError(null);
  }, [client, loadAccounts]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    void Promise.all([
      client.query(listProvidersQuery, {}, { requestPolicy: "network-only" }).toPromise(),
      loadAccounts(() => !cancelled),
    ]).then(([result]) => {
      if (cancelled) return;
      if (result.error) throw result.error;
      setProviders((result.data?.listProviders ?? []) as ListProviderManifest[]);
    }).catch((reason) => {
      if (!cancelled) setError(userFacingGraphQlErrorMessage(reason, t("status.failedToLoad")));
    }).finally(() => {
      if (!cancelled) setLoading(false);
    });
    return () => { cancelled = true; };
  }, [client, loadAccounts, retry, t]);

  const personalProviders = providers.filter((provider) =>
    provider.groups.some((group) => group.items.some((item) => item.personal)),
  );
  return (
    <div className="space-y-4 pt-4 text-sm text-[var(--scry-body)]">
    <SettingsProfileCard
      id="settings-profile-list-accounts"
      title={t("profile.listAccounts")}
      action={
        <Button asChild size="sm" variant="link">
          <Link to={buildListsPath("personal")}>{t("nav.lists")}</Link>
        </Button>
      }
    >
      {personalProviders.length > 0 ? <p className="text-sm text-[var(--scry-muted3)]">{t("lists.accounts.copy")}</p> : null}
      {loading ? (
        <p role="status" className="flex items-center gap-2 text-sm"><LoadingMark className="h-4 w-4" />{t("label.loading")}</p>
      ) : error ? (
        <div role="alert" className="space-y-2">
          <p className="text-sm text-[var(--scry-danger-text)]">{error}</p>
          <Button size="sm" variant="outline" onClick={() => setRetry((value) => value + 1)}>{t("lists.action.retry")}</Button>
        </div>
      ) : personalProviders.length === 0 ? <ListProviderSetup /> : (
        <PersonalAccountConnections
          providers={personalProviders}
          accounts={accounts}
          linkingProvider={accountLink.provider}
          onLink={(provider) => void accountLink.start(provider)}
        />
      )}
      {accountLink.error ? <p role="alert" className="text-sm text-[var(--scry-danger-text)]">{accountLink.error}</p> : null}
      {accountLink.provider ? (
        <div role="status" className="flex flex-wrap items-center gap-2 text-sm">
          <LoadingMark className="h-4 w-4" />{t("lists.accounts.waiting")}
          <Button size="sm" variant="outline" onClick={accountLink.cancel}>{t("label.cancel")}</Button>
        </div>
      ) : null}
    </SettingsProfileCard>
    {canManagePlugins ? (
      <FilteredPluginList
        family="LIST"
        title={t("lists.plugins.heading")}
        refreshProviderOptions={refreshProviders}
      />
    ) : null}
    </div>
  );
}

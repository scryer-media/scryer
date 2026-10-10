import * as React from "react";
import { Link } from "react-router";
import { LoadingMark } from "@/components/common/loading-mark";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { useTranslate } from "@/lib/context/translate-context";
import type { ListAccount, ListProviderItem, ListProviderManifest, ListSourceDraft } from "@/lib/types/lists";
import { buildViewPath } from "@/lib/utils/routing";
import { ProviderTile } from "./provider-tile";
import { ListProviderSetup } from "./list-provider-setup";

type Props = {
  providers: ListProviderManifest[];
  accounts: ListAccount[];
  managedAccount: ListAccount | null;
  accountLoading: boolean;
  busyIds: ReadonlySet<string>;
  linkingProvider: string | null;
  linkError: string | null;
  onLink: (provider: string) => void;
  onCancelLink: () => void;
  onManage: (account: ListAccount | null) => void;
  onUnlink: (account: ListAccount) => Promise<boolean>;
  onFollow: (manifest: ListProviderManifest, item: ListProviderItem, source: ListSourceDraft, name: string) => void;
};

type ConnectionsProps = Pick<Props, "providers" | "accounts" | "linkingProvider" | "onLink"> & {
  renderActions?: (account: ListAccount) => React.ReactNode;
};

export function PersonalAccountConnections(props: ConnectionsProps) {
  const t = useTranslate();
  return (
    <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
      {props.providers.flatMap((provider) => {
        const linkedAccounts = props.accounts.filter((entry) => entry.provider === provider.providerType);
        return (linkedAccounts.length ? linkedAccounts : [null]).map((linked) => ({ provider, linked }));
      }).map(({ provider, linked }) => (
        <Card key={linked?.id ?? provider.providerType} id={`lists-account-${linked?.id ?? provider.providerType}`}>
          <CardContent>
            <div className="flex items-center gap-3">
              <ProviderTile provider={provider} />
              <div className="min-w-0 flex-1">
                <h3 className="font-semibold">{provider.name}</h3>
                {linked ? <p className="truncate text-sm text-muted-foreground">{linked.displayName ?? linked.username ?? linked.externalUserId}</p> : null}
              </div>
              {linked ? <Badge tone={linked.status === "ACTIVE" ? "positive" : "warning"}>{t(linked.status === "ACTIVE" ? "lists.accounts.connected" : "lists.accounts.reconnect")}</Badge> : null}
            </div>
            {linked?.errorMessage ? <p className="text-sm text-destructive">{linked.errorMessage}</p> : null}
            <div className="flex flex-wrap gap-2">
              {linked ? props.renderActions?.(linked) : null}
              <Button size="sm" variant={linked ? "outline" : "default"} disabled={!!props.linkingProvider} onClick={() => props.onLink(provider.providerType)}>
                {t(linked ? "lists.accounts.reconnect" : "lists.accounts.connect")}
              </Button>
            </div>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

export function PersonalAccounts(props: Props) {
  const t = useTranslate();
  const [confirmUnlink, setConfirmUnlink] = React.useState(false);
  const catalog = props.providers.filter((provider) => provider.groups.some((group) => group.items.some((item) => item.personal)));
  const account = props.managedAccount;
  const manifest = catalog.find((provider) => provider.providerType === account?.provider);
  const items = manifest?.groups.flatMap((group) => group.items).filter((item) => item.personal) ?? [];
  const choices = account ? [
    ...account.ownedLists.map((list) => ({ key: `list:${list.id}`, label: list.name, sourceType: list.sourceType, params: list.params, kinds: list.kinds })),
    ...account.statuses.map((status) => ({ key: `status:${status.key}`, label: status.label, sourceType: status.sourceType, params: status.params, kinds: status.kinds })),
  ] : [];

  return (
    <section id="lists-personal-accounts" className="space-y-3">
      <h2 className="font-display text-[17px] font-bold">{t("lists.accounts.heading")}</h2>
      <p className="text-sm text-[var(--scry-muted)]">{t("lists.accounts.copy")}</p>
      <Button asChild size="sm" variant="outline">
        <Link id="lists-profile-link" to={buildViewPath("settings", "profile")}>
          {t("lists.accounts.linkProfile")}
        </Link>
      </Button>
      {props.linkError ? <p role="alert" className="text-sm text-[var(--scry-danger-text)]">{props.linkError}</p> : null}
      {props.linkingProvider ? (
        <div role="status" className="flex items-center gap-2 text-sm">
          <LoadingMark className="h-4 w-4" />{t("lists.accounts.waiting")}
          <Button size="sm" variant="outline" onClick={props.onCancelLink}>{t("label.cancel")}</Button>
        </div>
      ) : null}
      <PersonalAccountConnections
        providers={catalog}
        accounts={props.accounts}
        linkingProvider={props.linkingProvider}
        onLink={props.onLink}
        renderActions={(linked) => (
          <>
            <Button size="sm" variant="outline" onClick={() => { setConfirmUnlink(false); props.onManage(linked); }}>{t("lists.accounts.manage")}</Button>
            <Button size="sm" variant="outline" onClick={() => { setConfirmUnlink(true); props.onManage(linked); }}>{t("lists.accounts.unlink")}</Button>
          </>
        )}
      />
      {catalog.length === 0 ? <ListProviderSetup /> : null}
      <Dialog open={!!account} onOpenChange={(open) => { if (!open) props.onManage(null); }}>
        {account ? (
          <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
            <DialogHeader>
              <DialogTitle>{t("lists.accounts.manageTitle", { provider: manifest?.name ?? account.provider })}</DialogTitle>
              <DialogDescription>{t("lists.accounts.private")}</DialogDescription>
            </DialogHeader>
            {props.accountLoading ? <LoadingMark className="h-5 w-5" /> : (
              <div className="space-y-4">
                {items.filter((item) => !choices.some((choice) => choice.sourceType === item.sourceType)).map((item) => (
                  <div key={item.id} className="flex items-center gap-3 rounded-lg border p-3">
                    <div className="min-w-0 flex-1"><p className="font-medium">{item.name}</p><p className="text-sm text-muted-foreground">{item.description}</p></div>
                    <Button size="sm" variant="outline" onClick={() => {
                      if (!manifest) return;
                      props.onFollow(manifest, item, { provider: account.provider, sourceType: item.sourceType, params: [], url: null, credentialId: account.id }, item.name);
                      props.onManage(null);
                    }}>{t("lists.catalog.follow")}</Button>
                  </div>
                ))}
                {choices.map((choice) => {
                  const item = items.find((entry) => entry.sourceType === choice.sourceType);
                  if (!item || !manifest) return null;
                  return (
                    <div key={choice.key} className="flex items-center gap-3 rounded-lg border p-3">
                      <p className="min-w-0 flex-1 text-sm font-medium">{choice.label}</p>
                      <Button size="sm" variant="outline" onClick={() => {
                        props.onFollow(manifest, { ...item, kinds: choice.kinds }, { provider: account.provider, sourceType: choice.sourceType, params: choice.params, fixedParams: true, url: null, credentialId: account.id }, choice.label);
                        props.onManage(null);
                      }}>{t("lists.catalog.follow")}</Button>
                    </div>
                  );
                })}
              </div>
            )}
            <div className="space-y-2 border-t pt-4">
              <p className="text-sm text-muted-foreground">{t("lists.accounts.unlinkHelp")}</p>
              {confirmUnlink ? (
                <div className="flex gap-2">
                  <Button size="sm" disabled={props.busyIds.has(account.id)} onClick={() => void props.onUnlink(account)}>{t("lists.accounts.confirmUnlink")}</Button>
                  <Button size="sm" variant="outline" onClick={() => setConfirmUnlink(false)}>{t("label.cancel")}</Button>
                </div>
              ) : <Button size="sm" variant="outline" onClick={() => setConfirmUnlink(true)}>{t("lists.accounts.unlink")}</Button>}
            </div>
          </DialogContent>
        ) : null}
      </Dialog>
    </section>
  );
}

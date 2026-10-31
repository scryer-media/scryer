import * as React from "react";
import { LoadingMark } from "@/components/common/loading-mark";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { useTranslate } from "@/lib/context/translate-context";
import type { ListAccountMemberKey } from "@/lib/hooks/use-list-account-link";
import type {
  ListAccount,
  ListMemberKey,
  ListPreview,
  ListProviderItem,
  ListProviderManifest,
  ListSourceDraft,
  ListSubscription,
} from "@/lib/types/lists";
import {
  listAccountKeyEntry,
  listAccountKeyHelpUrl,
} from "@/lib/utils/list-account-link";
import { isListSourceFollowed, listAccountLabel } from "@/lib/utils/lists";
import { CatalogListTable, type CatalogListRow } from "./catalog-list-table";
import { ListProviderSetup } from "./list-provider-setup";
import { ProviderRail, ProviderRailEntry } from "./provider-rail";
import { ProviderTile } from "./provider-tile";

type Props = {
  providers: ListProviderManifest[];
  accounts: ListAccount[];
  /** The member's own follows, so a list already followed is not offered again. */
  subscriptions: ListSubscription[];
  /** The open provider's accounts as read from the provider, each carrying its lists. */
  managedAccounts: ListAccount[];
  accountLoading: boolean;
  /** Why an account's lists could not be read just now, by account id. */
  accountReadErrors: Readonly<Record<string, string>>;
  busyIds: ReadonlySet<string>;
  linkingProvider: string | null;
  /** The link in progress is a key being checked, with no provider window to finish in. */
  checkingKey: boolean;
  linkError: string | null;
  /** Starts a link: by the provider's sign-in, or with the key the member typed. */
  onLink: (provider: string, memberKey?: ListAccountMemberKey) => void;
  onCancelLink: () => void;
  /** Reads the lists of the open provider's accounts from the provider. */
  onManage: (accounts: ListAccount[]) => void;
  onUnlink: (account: ListAccount) => Promise<boolean>;
  onFollow: (
    manifest: ListProviderManifest,
    item: ListProviderItem,
    source: ListSourceDraft,
    name: string,
  ) => void;
  /** Reads one of the account's lists as it stands, for the preview in its row. */
  onPreviewSource: (source: ListSourceDraft) => Promise<ListPreview | null>;
};

type ConnectionsProps = Pick<
  Props,
  "providers" | "accounts" | "linkingProvider" | "onLink"
> & {
  renderActions?: (account: ListAccount) => React.ReactNode;
};

/** An open key form: whose it is, and whether its key connects a further account or replaces this one's. */
type KeyFormTarget = { key: string; another: boolean };

/**
 * Where a member pastes the key a provider issues them, for a provider that
 * connects by key instead of a sign-in window. The key leaves this form only
 * in the request that stores it.
 */
function MemberKeyForm(props: {
  id: string;
  memberKey: ListMemberKey;
  replacing: boolean;
  onSubmit: (memberKey: ListAccountMemberKey) => void;
  onCancel: () => void;
}) {
  const t = useTranslate();
  const [typed, setTyped] = React.useState("");
  const [missing, setMissing] = React.useState(false);
  const helpUrl = listAccountKeyHelpUrl(props.memberKey.helpUrl);
  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    const entry = listAccountKeyEntry(props.memberKey, typed);
    if (entry.kind === "missing") {
      setMissing(true);
      return;
    }
    props.onSubmit({ key: entry.key, approval: entry.approval });
  };
  return (
    <form
      id={props.id}
      className="space-y-2 rounded-[12px] border border-[var(--scry-border3)] p-3 text-sm"
      onSubmit={submit}
    >
      <label className="block space-y-1">
        <span>{props.memberKey.label}</span>
        <Input
          id={`${props.id}-input`}
          type="password"
          ignorePasswordManagers
          value={typed}
          aria-invalid={missing || undefined}
          onChange={(event) => {
            setTyped(event.target.value);
            setMissing(false);
          }}
        />
      </label>
      {missing ? (
        <p role="alert" className="text-[var(--scry-danger-text)]">
          {t("lists.accounts.key.missing")}
        </p>
      ) : null}
      {props.memberKey.helpText ? (
        <p className="text-[var(--scry-muted)]">{props.memberKey.helpText}</p>
      ) : null}
      {helpUrl ? (
        <a
          className="inline-block underline"
          href={helpUrl}
          target="_blank"
          rel="noreferrer noopener"
        >
          {t("lists.accounts.key.where")}
        </a>
      ) : null}
      {props.memberKey.approval ? (
        <p className="text-[var(--scry-muted)]">
          {t("lists.accounts.key.approval")}
        </p>
      ) : null}
      {props.replacing ? (
        <p className="text-[var(--scry-muted)]">
          {t("lists.accounts.key.replace")}
        </p>
      ) : null}
      <p className="text-[var(--scry-muted)]">
        {t("lists.accounts.key.private")}
      </p>
      <div className="flex gap-2">
        <Button id={`${props.id}-save`} type="submit" size="sm">
          {t("lists.accounts.connect")}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={props.onCancel}
        >
          {t("label.cancel")}
        </Button>
      </div>
    </form>
  );
}

export function PersonalAccountConnections(props: ConnectionsProps) {
  const t = useTranslate();
  // The card whose key form is open, for a provider that connects by key.
  const [keyFormFor, setKeyFormFor] = React.useState<KeyFormTarget | null>(
    null,
  );
  return (
    <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
      {props.providers
        .flatMap((provider) => {
          const linkedAccounts = props.accounts.filter(
            (entry) => entry.provider === provider.providerType,
          );
          return (linkedAccounts.length ? linkedAccounts : [null]).map(
            (linked) => ({ provider, linked }),
          );
        })
        .map(({ provider, linked }) => ({
          provider,
          linked,
          key: linked?.id ?? provider.providerType,
        }))
        .map(({ provider, linked, key }) => (
          <Card key={key} id={`lists-account-${key}`}>
            <CardContent>
              <div className="flex items-center gap-3">
                <ProviderTile provider={provider} />
                <div className="min-w-0 flex-1">
                  <h3 className="font-semibold">{provider.name}</h3>
                  {linked ? (
                    <p className="truncate text-sm text-muted-foreground">
                      {listAccountLabel(linked)}
                    </p>
                  ) : null}
                </div>
                {linked ? (
                  <Badge
                    tone={linked.status === "ACTIVE" ? "positive" : "warning"}
                  >
                    {t(
                      linked.status === "ACTIVE"
                        ? "lists.accounts.connected"
                        : "lists.accounts.reconnect",
                    )}
                  </Badge>
                ) : null}
              </div>
              {linked?.errorMessage ? (
                <p className="text-sm text-destructive">
                  {linked.errorMessage}
                </p>
              ) : null}
              <div className="flex flex-wrap gap-2">
                {linked ? props.renderActions?.(linked) : null}
                <Button
                  size="sm"
                  variant={linked ? "outline" : "default"}
                  disabled={!!props.linkingProvider}
                  onClick={() =>
                    provider.memberKey
                      ? setKeyFormFor({ key, another: false })
                      : props.onLink(provider.providerType)
                  }
                >
                  {t(
                    linked
                      ? "lists.accounts.reconnect"
                      : "lists.accounts.connect",
                  )}
                </Button>
                {linked ? (
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={!!props.linkingProvider}
                    onClick={() =>
                      provider.memberKey
                        ? setKeyFormFor({ key, another: true })
                        : props.onLink(provider.providerType)
                    }
                  >
                    {t("lists.accounts.connectAnother")}
                  </Button>
                ) : null}
              </div>
              {provider.memberKey && keyFormFor?.key === key ? (
                <MemberKeyForm
                  id={`lists-account-key-${key}`}
                  memberKey={provider.memberKey}
                  replacing={!!linked && !keyFormFor.another}
                  onCancel={() => setKeyFormFor(null)}
                  onSubmit={(memberKey) => {
                    setKeyFormFor(null);
                    props.onLink(provider.providerType, memberKey);
                  }}
                />
              ) : null}
            </CardContent>
          </Card>
        ))}
    </div>
  );
}

/** A provider in the rail, with every account the member has linked to it. */
type ProviderEntry = { provider: ListProviderManifest; linked: ListAccount[] };

/**
 * The member's own provider accounts: a rail of the providers that offer
 * personal lists, and for the open one its connections and the lists every
 * account linked to it can follow.
 */
export function PersonalAccounts(props: Props) {
  const t = useTranslate();
  const { onManage } = props;
  const entries = props.providers
    .filter((provider) =>
      provider.groups.some((group) =>
        group.items.some((item) => item.personal),
      ),
    )
    .map((provider): ProviderEntry => ({
      provider,
      linked: props.accounts.filter(
        (entry) => entry.provider === provider.providerType,
      ),
    }));
  const [picked, setPicked] = React.useState<string | null>(null);
  const selected =
    entries.find((entry) => entry.provider.providerType === picked) ??
    entries[0] ??
    null;
  // The link in progress adds a further account, which the provider settles by who is signed in there.
  const [linkingAnother, setLinkingAnother] = React.useState(false);
  const [unlinkingId, setUnlinkingId] = React.useState<string | null>(null);
  // The account or provider whose key form is open, for a provider that connects by key.
  const [keyFormFor, setKeyFormFor] = React.useState<KeyFormTarget | null>(
    null,
  );

  // The open provider's accounts have their lists read when it is opened, and again once one is connected, reconnected or unlinked.
  const linked = selected?.linked ?? [];
  const linkedSignature = linked
    .map((entry) => `${entry.id}:${entry.status}`)
    .join(",");
  const openAccounts = React.useRef<() => void>(() => {});
  React.useEffect(() => {
    openAccounts.current = () => onManage(linked);
  });
  React.useEffect(() => {
    openAccounts.current();
  }, [selected?.provider.providerType, linkedSignature]);

  const manifest = selected?.provider ?? null;
  const providerType = manifest?.providerType ?? "";
  const memberKey = manifest?.memberKey ?? null;
  const items =
    manifest?.groups
      .flatMap((group) => group.items)
      .filter((item) => item.personal) ?? [];
  // A loaded account carries its lists; until it arrives the linked one stands in.
  const accounts = linked.map(
    (entry) =>
      props.managedAccounts.find((loaded) => loaded.id === entry.id) ?? entry,
  );
  const several = accounts.length > 1;
  const rows: CatalogListRow[] = [];
  if (manifest) {
    for (const account of accounts) {
      const choices = [
        ...account.ownedLists.map((list) => ({
          key: `list:${list.id}`,
          label: list.name,
          sourceType: list.sourceType,
          params: list.params,
          kinds: list.kinds,
        })),
        ...account.statuses.map((status) => ({
          key: `status:${status.key}`,
          label: status.label,
          sourceType: status.sourceType,
          params: status.params,
          kinds: status.kinds,
        })),
      ];
      const followed = (
        sourceType: string,
        params: ListSourceDraft["params"],
      ) =>
        isListSourceFollowed(props.subscriptions, {
          provider: account.provider,
          sourceType,
          params,
          accountId: account.id,
        });
      const followId = (key: string) =>
        `lists-account-follow-${account.id}-${key.replace(/[^A-Za-z0-9_-]/g, "-")}`;
      for (const item of items) {
        if (choices.some((choice) => choice.sourceType === item.sourceType))
          continue;
        const source: ListSourceDraft = {
          provider: account.provider,
          sourceType: item.sourceType,
          params: [],
          url: null,
          credentialId: account.id,
        };
        rows.push({
          key: `${account.id}:item:${item.id}`,
          name: item.name,
          description: item.description,
          kinds: item.kinds,
          account: listAccountLabel(account),
          intervalSeconds: item.defaultIntervalSeconds,
          followed: followed(item.sourceType, []),
          followId: followId(item.id),
          onFollow: () => props.onFollow(manifest, item, source, item.name),
          // A list that needs a value has nothing to read until it is given one.
          onPreview: item.params.some((param) => param.required)
            ? undefined
            : () => props.onPreviewSource(source),
        });
      }
      for (const choice of choices) {
        const item = items.find(
          (entry) => entry.sourceType === choice.sourceType,
        );
        if (!item) continue;
        const source: ListSourceDraft = {
          provider: account.provider,
          sourceType: choice.sourceType,
          params: choice.params,
          fixedParams: true,
          url: null,
          credentialId: account.id,
        };
        rows.push({
          key: `${account.id}:${choice.key}`,
          name: choice.label,
          kinds: choice.kinds,
          account: listAccountLabel(account),
          intervalSeconds: item.defaultIntervalSeconds,
          followed: followed(choice.sourceType, choice.params),
          followId: followId(choice.key),
          onFollow: () =>
            props.onFollow(
              manifest,
              { ...item, kinds: choice.kinds },
              source,
              choice.label,
            ),
          onPreview: () => props.onPreviewSource(source),
        });
      }
    }
  }
  // Starts a link with the open provider: a further account, or the one already there.
  const link = (another: boolean, entered?: ListAccountMemberKey) => {
    setLinkingAnother(another);
    props.onLink(providerType, entered);
  };
  const keyForm = (key: string, replacing: boolean) =>
    memberKey && keyFormFor?.key === key && !props.linkingProvider ? (
      <MemberKeyForm
        id={`lists-account-key-${key}`}
        memberKey={memberKey}
        replacing={replacing}
        onCancel={() => setKeyFormFor(null)}
        onSubmit={(entered) => {
          setKeyFormFor(null);
          link(keyFormFor.another, entered);
        }}
      />
    ) : null;
  const connectAnother = (
    <Button
      id={`lists-account-add-${providerType}`}
      size="sm"
      variant="outline"
      disabled={!!props.linkingProvider}
      onClick={() =>
        memberKey
          ? setKeyFormFor({ key: providerType, another: true })
          : link(true)
      }
    >
      {t("lists.accounts.connectAnother")}
    </Button>
  );

  return (
    <section id="lists-personal-accounts" className="space-y-3">
      <h2 className="flex min-h-8 items-center font-display text-[17px] font-bold">
        {t("lists.accounts.heading")}
      </h2>
      <p className="text-sm text-[var(--scry-muted)]">
        {t("lists.accounts.copy")}
      </p>
      {entries.length === 0 ? <ListProviderSetup /> : null}
      {selected ? (
        <div
          id="lists-accounts"
          className="grid gap-4 md:grid-cols-[220px_minmax(0,1fr)]"
        >
          <ProviderRail label={t("lists.catalog.providers")}>
            {entries.map((entry) => (
              <ProviderRailEntry
                key={entry.provider.providerType}
                id={`lists-account-${entry.provider.providerType}`}
                active={entry === selected}
                onSelect={() => setPicked(entry.provider.providerType)}
                icon={<ProviderTile provider={entry.provider} size="sm" />}
                name={entry.provider.name}
                detail={
                  entry.linked.length
                    ? entry.linked.map(listAccountLabel).join(", ")
                    : t("lists.accounts.notConnected")
                }
              />
            ))}
          </ProviderRail>

          <div
            id={`lists-account-panel-${providerType}`}
            className="min-w-0 space-y-3"
          >
            {accounts.map((account) => {
              const confirmingUnlink = unlinkingId === account.id;
              return (
                <div
                  key={account.id}
                  id={`lists-account-connection-${account.id}`}
                  className="space-y-3"
                >
                  <div className="flex flex-wrap items-center gap-2">
                    {several ? (
                      <span
                        data-ui="list-account-name"
                        className="text-sm font-semibold text-[var(--scry-ink2)]"
                      >
                        {listAccountLabel(account)}
                      </span>
                    ) : null}
                    <Badge
                      tone={
                        account.status === "ACTIVE" ? "positive" : "warning"
                      }
                    >
                      {t(
                        account.status === "ACTIVE"
                          ? "lists.accounts.connected"
                          : "lists.accounts.needsReconnect",
                      )}
                    </Badge>
                    <Button
                      id={`lists-account-link-${account.id}`}
                      size="sm"
                      variant={
                        account.status === "ACTIVE" ? "outline" : "primary"
                      }
                      disabled={!!props.linkingProvider}
                      onClick={() =>
                        memberKey
                          ? setKeyFormFor({ key: account.id, another: false })
                          : link(false)
                      }
                    >
                      {t("lists.accounts.reconnect")}
                    </Button>
                    {several ? null : connectAnother}
                    {confirmingUnlink ? null : (
                      <Button
                        id={`lists-account-unlink-${account.id}`}
                        size="sm"
                        variant="outline"
                        onClick={() => setUnlinkingId(account.id)}
                      >
                        {t("lists.accounts.unlink")}
                      </Button>
                    )}
                  </div>
                  {account.errorMessage ? (
                    <p className="text-sm text-[var(--scry-danger-text)]">
                      {account.errorMessage}
                    </p>
                  ) : null}
                  {props.accountReadErrors[account.id] ? (
                    <p
                      data-ui="list-account-read-error"
                      className="text-[12.5px] text-[var(--scry-muted)]"
                    >
                      {props.accountReadErrors[account.id]}
                    </p>
                  ) : null}
                  {keyForm(account.id, true)}
                  {confirmingUnlink ? (
                    <div className="space-y-2 rounded-[12px] border border-[var(--scry-border3)] p-3">
                      <p className="text-sm text-[var(--scry-muted)]">
                        {t("lists.accounts.unlinkHelp")}
                      </p>
                      <div className="flex gap-2">
                        <Button
                          id={`lists-account-unlink-confirm-${account.id}`}
                          size="sm"
                          disabled={props.busyIds.has(account.id)}
                          onClick={() =>
                            void props
                              .onUnlink(account)
                              .then(() => setUnlinkingId(null))
                          }
                        >
                          {t("lists.accounts.confirmUnlink")}
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => setUnlinkingId(null)}
                        >
                          {t("label.cancel")}
                        </Button>
                      </div>
                    </div>
                  ) : null}
                </div>
              );
            })}
            {accounts.length === 0 ? (
              <div>
                <Button
                  id={`lists-account-link-${providerType}`}
                  size="sm"
                  variant="primary"
                  disabled={!!props.linkingProvider}
                  onClick={() =>
                    memberKey
                      ? setKeyFormFor({ key: providerType, another: false })
                      : link(false)
                  }
                >
                  {t("lists.accounts.connect")}
                </Button>
              </div>
            ) : null}
            {several ? <div>{connectAnother}</div> : null}
            {keyForm(providerType, false)}
            {props.linkError ? (
              <p
                role="alert"
                className="text-sm text-[var(--scry-danger-text)]"
              >
                {props.linkError}
              </p>
            ) : null}
            {props.linkingProvider ? (
              <div role="status" className="flex items-center gap-2 text-sm">
                <LoadingMark className="h-4 w-4" />
                {t(
                  props.checkingKey
                    ? "lists.accounts.key.checking"
                    : "lists.accounts.waiting",
                )}
                <Button
                  size="sm"
                  variant="outline"
                  onClick={props.onCancelLink}
                >
                  {t("label.cancel")}
                </Button>
              </div>
            ) : null}
            {props.linkingProvider && linkingAnother && !props.checkingKey ? (
              <p className="text-[12.5px] text-[var(--scry-muted)]">
                {t("lists.accounts.connectAnotherHint")}
              </p>
            ) : null}
            {accounts.length === 0 ? null : props.accountLoading ? (
              <p
                role="status"
                className="flex items-center gap-2 text-[12.5px] text-[var(--scry-muted)]"
              >
                <LoadingMark className="h-4 w-4" />
                {t("label.loading")}
              </p>
            ) : rows.length > 0 ? (
              <CatalogListTable
                key={providerType}
                id={`lists-account-lists-${providerType}`}
                providerType={providerType}
                rows={rows}
              />
            ) : null}
          </div>
        </div>
      ) : null}
    </section>
  );
}

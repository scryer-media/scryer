import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { useClient } from "urql";
import {
  Ban,
  CheckCircle2,
  Clipboard,
  Edit,
  KeyRound,
  Lock,
  Plus,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { LoadingMark } from "@/components/common/loading-mark";
import { TotpCodeForm } from "@/components/auth/totp-code-form";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox, CheckboxField } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableActionsCell,
  TableActionsHead,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { IndexerCategoryPicker } from "@/components/views/media-content/indexer-category-picker";
import { SortableHead } from "@/components/views/lists/sortable-head";
import { FilteredPluginList } from "@/components/views/settings/filtered-plugin-list";
import { INDEXER_NARROW_CELL_CLASS as NARROW_CELL } from "@/components/views/settings/settings-indexers-section";
import { cn } from "@/lib/utils";
import { SETTINGS_REFERENCE_SLOT_ID } from "@/components/containers/settings/settings-container";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { useAuth, type AuthUser } from "@/lib/hooks/use-auth";
import { mfaVerifyStepUpMutation } from "@/lib/graphql/mutations";
import { indexersQuery, tlsSettingsQuery } from "@/lib/graphql/queries";
import {
  createIndexerClientMutation,
  indexerClientsQuery,
  revokeIndexerClientKeyMutation,
  rotateIndexerClientKeyMutation,
  updateIndexerClientMutation,
  type IndexerClientConfiguration,
  type IndexerClientProtocol,
  type IndexerClientRecord,
} from "@/lib/graphql/indexer-clients";
import type { IndexerRecord } from "@/lib/types";
import type { ServiceSettings } from "@/lib/types/settings";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  indexerClientAccessRefusal,
  indexerClientErrorMessage,
} from "@/lib/utils/indexer-client-error";
import { indexerClientOutcomeMessage } from "@/lib/utils/indexer-client-outcome";
import { withMfaStepUp } from "@/lib/utils/with-mfa-step-up";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import { indexerClientEndpointUrl } from "@/lib/utils/indexer-client-endpoint";
import {
  sortIndexerClients,
  type IndexerClientSort,
  type IndexerClientSortKey,
} from "@/lib/utils/indexer-client-sort";

/** Below 1279px these tables become one card per row, like the indexers table. */
const NARROW_TABLE =
  "max-[1279px]:block max-[1279px]:[&_thead]:hidden max-[1279px]:[&_tbody]:block max-[1279px]:p-2";
const NARROW_ROW =
  "max-[1279px]:mb-2 max-[1279px]:block max-[1279px]:overflow-hidden max-[1279px]:rounded-lg max-[1279px]:border max-[1279px]:border-border last:max-[1279px]:mb-0";

const PROTOCOLS: readonly IndexerClientProtocol[] = ["NEWZNAB", "TORZNAB"];
const PROTOCOL_LABEL_KEYS: Readonly<Record<IndexerClientProtocol, string>> = {
  NEWZNAB: "indexerClients.protocol.newznab",
  TORZNAB: "indexerClients.protocol.torznab",
};

function EndpointField({
  id,
  label,
  value,
  copied,
  onCopy,
  t,
}: {
  id: string;
  label: string;
  value: string;
  copied: boolean;
  onCopy: () => void;
  t: (key: string) => string;
}) {
  return (
    <div className="flex w-fit min-w-0 max-w-full items-center">
      <Input
        id={id}
        aria-label={label}
        readOnly
        value={value}
        size={value.length + 2}
        onFocus={(event) => event.currentTarget.select()}
        className="h-9 w-auto min-w-0 max-w-full rounded-r-none font-[var(--font-code)] text-sm text-[var(--scry-ink2)]"
      />
      <IconButton
        id={`${id}-copy`}
        label={copied ? t("label.copied") : t("label.copy")}
        tone={copied ? "enabled" : "neutral"}
        className="-ml-px h-9 shrink-0 rounded-l-none"
        onClick={onCopy}
      >
        {copied ? (
          <CheckCircle2 className="h-4 w-4" />
        ) : (
          <Clipboard className="h-4 w-4" />
        )}
      </IconButton>
    </div>
  );
}

const emptyDraft = (): IndexerClientConfiguration => ({
  name: "",
  enabled: true,
  protocols: ["NEWZNAB"],
  grants: [],
});
const configurationOf = (
  client: IndexerClientRecord,
): IndexerClientConfiguration => ({
  name: client.name,
  enabled: client.enabled,
  protocols: [...client.protocols],
  grants: client.grants.map((grant) => ({
    indexerId: grant.indexerId,
    categories: grant.categories === null ? null : [...grant.categories],
  })),
});

type KeyChange = {
  client: IndexerClientRecord;
  operation: "rotate" | "revoke";
};
type MutationData = {
  createIndexerClient?: { apiKey: string; client: { id: string } };
  rotateIndexerClientKey?: { apiKey: string; client: { id: string } };
};

export function SettingsIndexerClientsContainer() {
  const api = useClient();
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const { adoptSession } = useAuth();
  const [clients, setClients] = useState<IndexerClientRecord[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [sources, setSources] = useState<IndexerRecord[]>([]);
  const [sort, setSort] = useState<IndexerClientSort>({
    key: "name",
    descending: false,
  });
  const sortedClients = useMemo(
    () => (clients ? sortIndexerClients(clients, sort) : null),
    [clients, sort],
  );
  const sortBy = (key: IndexerClientSortKey) =>
    setSort((current) => ({
      key,
      descending: current.key === key ? !current.descending : false,
    }));
  const sortHead = (
    key: IndexerClientSortKey,
    label: string,
    className: string,
  ) => (
    <SortableHead
      id={selectorId("settings-indexer-clients-sort", key)}
      label={label}
      active={sort.key === key}
      descending={sort.descending}
      onSort={() => sortBy(key)}
      className={className}
    />
  );
  const [draft, setDraft] = useState(emptyDraft);
  const [editorOpen, setEditorOpen] = useState(false);
  const [editing, setEditing] = useState<IndexerClientRecord | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [accessRefusal, setAccessRefusal] = useState<string | null>(null);
  /** A new key is shown once, under the row of the client it belongs to. */
  const [revealed, setRevealed] = useState<{
    clientId: string;
    apiKey: string;
  } | null>(null);
  const [revealedCopied, setRevealedCopied] = useState(false);
  const [confirmation, setConfirmation] = useState<KeyChange | null>(null);
  const [mfaOpen, setMfaOpen] = useState(false);
  const [mfaCode, setMfaCode] = useState("");
  const [mfaBusy, setMfaBusy] = useState(false);
  const [mfaError, setMfaError] = useState<string | null>(null);
  const verification = useRef<((token: string | null) => void) | null>(null);
  const mounted = useRef(false);
  const loadVersion = useRef(0);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      verification.current?.(null);
      verification.current = null;
    };
  }, []);

  const load = useCallback(async () => {
    const version = ++loadVersion.current;
    setLoading(true);
    try {
      const result = await api
        .query<{ indexerClients: IndexerClientRecord[] }>(
          indexerClientsQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (!mounted.current || version !== loadVersion.current) return;
      if (result.error || !result.data) {
        setClients(null);
        throw result.error;
      }
      const indexers = await api
        .query<{ indexers: IndexerRecord[] }>(
          indexersQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (!mounted.current || version !== loadVersion.current) return;
      if (indexers.error || !indexers.data) {
        setClients(null);
        throw indexers.error;
      }
      setSources(
        indexers.data.indexers.filter(
          (source) => !source.supportsManagedChildrenSync,
        ),
      );
      setAccessRefusal(null);
      setClients(result.data.indexerClients);
    } catch (failure) {
      if (!mounted.current || version !== loadVersion.current) return;
      const refusal = indexerClientAccessRefusal(failure, t);
      setAccessRefusal(refusal);
      if (!refusal) setError(indexerClientErrorMessage(failure, t));
    } finally {
      if (mounted.current && version === loadVersion.current) setLoading(false);
    }
  }, [api, t]);
  useEffect(() => {
    void load();
  }, [load]);
  const [pluginsTarget, setPluginsTarget] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setPluginsTarget(document.getElementById(SETTINGS_REFERENCE_SLOT_ID));
  }, []);
  const refreshAfterPluginChange = useCallback(async () => {
    await load();
  }, [load]);
  const [copiedEndpoint, setCopiedEndpoint] = useState<string | null>(null);
  const [endpointBase, setEndpointBase] = useState<Pick<
    ServiceSettings,
    "publicUrl" | "basePath"
  > | null>(null);
  useEffect(() => {
    let cancelled = false;
    void api
      .query<{ serviceSettings: ServiceSettings }>(
        tlsSettingsQuery,
        {},
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then((result) => {
        if (!cancelled && result.data)
          setEndpointBase(result.data.serviceSettings);
      });
    return () => {
      cancelled = true;
    };
  }, [api]);
  const endpointUrl = (protocol: IndexerClientProtocol) =>
    indexerClientEndpointUrl(
      protocol,
      endpointBase?.publicUrl ?? null,
      endpointBase?.basePath ?? getRuntimeBasePath(),
      window.location.origin,
    );
  const copyEndpoint = async (key: string, value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedEndpoint(key);
    } catch {
      setError(t("indexerClients.endpointCopyFailed"));
    }
  };

  const requestMfa = useCallback(
    () =>
      new Promise<string | null>((resolve) => {
        if (!mounted.current) {
          resolve(null);
          return;
        }
        verification.current = resolve;
        setMfaCode("");
        setMfaError(null);
        setMfaOpen(true);
      }),
    [],
  );
  const cancelMfa = () => {
    if (mfaBusy) return;
    verification.current?.(null);
    verification.current = null;
    setMfaOpen(false);
    setMfaCode("");
  };
  const verifyMfa = async () => {
    if (mfaBusy || !mfaCode.trim()) return;
    setMfaBusy(true);
    setMfaError(null);
    try {
      const result = await api
        .mutation<{
          mfaVerifyStepUp?: {
            token: string;
            user: AuthUser | null;
            persistSession: boolean;
          };
        }>(mfaVerifyStepUpMutation, { input: { code: mfaCode } })
        .toPromise();
      if (!mounted.current || !verification.current) return;
      const session = result.data?.mfaVerifyStepUp;
      if (result.error || !session) {
        setMfaError(t("settings.mfaStepUpFailed"));
        setMfaCode("");
        return;
      }
      adoptSession(session.token, session.user, session.persistSession);
      verification.current(session.token);
      verification.current = null;
      setMfaOpen(false);
      setMfaCode("");
    } catch {
      if (mounted.current) setMfaError(t("settings.mfaStepUpFailed"));
    } finally {
      if (mounted.current) setMfaBusy(false);
    }
  };

  const closeEditor = () => {
    setEditorOpen(false);
    setEditing(null);
    setDraft(emptyDraft());
  };

  const mutate = async (
    document: string,
    variables: Record<string, unknown>,
    closeOnSuccess = false,
  ) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    setRevealed(null);
    try {
      const result = await withMfaStepUp(
        (token) =>
          api
            .mutation<MutationData>(
              document,
              variables,
              token
                ? {
                    fetchOptions: {
                      headers: { Authorization: `Bearer ${token}` },
                    },
                  }
                : undefined,
            )
            .toPromise(),
        requestMfa,
      );
      if (!mounted.current || !result) return;
      if (result.error || !result.data) throw result.error;
      const issued =
        result.data.createIndexerClient ?? result.data.rotateIndexerClientKey;
      setRevealed(
        issued ? { clientId: issued.client.id, apiKey: issued.apiKey } : null,
      );
      setRevealedCopied(false);
      if (closeOnSuccess) closeEditor();
      await load();
    } catch (failure) {
      if (mounted.current) setError(indexerClientErrorMessage(failure, t));
    } finally {
      if (mounted.current) setBusy(false);
    }
  };

  const copyRevealed = async () => {
    if (!revealed) return;
    try {
      await navigator.clipboard.writeText(revealed.apiKey);
      setRevealedCopied(true);
    } catch {
      setError(t("error.fallback.unableToCopyTheAPIKey"));
    }
  };

  const setGrant = (id: string, categories: string[] | null | undefined) =>
    setDraft((current) => ({
      ...current,
      grants: [
        ...current.grants.filter((grant) => grant.indexerId !== id),
        ...(categories === undefined ? [] : [{ indexerId: id, categories }]),
      ],
    }));
  const startCreate = () => {
    setEditing(null);
    setDraft(emptyDraft());
    setRevealed(null);
    setEditorOpen(true);
  };
  const startEdit = (client: IndexerClientRecord) => {
    setEditing(client);
    setRevealed(null);
    setDraft(configurationOf(client));
    setEditorOpen(true);
  };
  const toggleEnabled = (client: IndexerClientRecord, enabled: boolean) =>
    void mutate(updateIndexerClientMutation, {
      id: client.id,
      expectedRevision: client.revision,
      input: { ...configurationOf(client), enabled },
    });

  return (
    <div id="settings-indexer-clients" className="flex flex-col gap-4 text-sm">
      {pluginsTarget
        ? createPortal(
            <FilteredPluginList
              family="INDEXER"
              refreshProviderOptions={refreshAfterPluginChange}
            />,
            pluginsTarget,
          )
        : null}

      <div
        id="settings-indexer-client-endpoints"
        className="rounded border border-border"
      >
        <div className="border-b border-border px-3 py-2">
          <CardTitle className="text-base">
            {t("indexerClients.endpoints")}
          </CardTitle>
        </div>
        <div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-4 gap-y-2 p-3">
          {PROTOCOLS.map((protocol) => (
            <Fragment key={protocol}>
              <span className="font-medium text-[var(--scry-ink2)]">
                {t(PROTOCOL_LABEL_KEYS[protocol])}
              </span>
              <EndpointField
                id={`indexer-client-endpoint-${protocol.toLowerCase()}`}
                label={t(PROTOCOL_LABEL_KEYS[protocol])}
                value={endpointUrl(protocol)}
                copied={copiedEndpoint === protocol}
                onCopy={() =>
                  void copyEndpoint(protocol, endpointUrl(protocol))
                }
                t={t}
              />
            </Fragment>
          ))}
        </div>
      </div>

      {error ? (
        <div
          id="settings-indexer-clients-error"
          role="alert"
          className="flex flex-wrap items-start justify-between gap-3 rounded-[12px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 text-sm text-[var(--scry-danger-text)]"
        >
          <span className="min-w-0 whitespace-pre-wrap break-words">
            {error}
          </span>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-auto shrink-0 px-2 py-0.5 text-xs"
            onClick={() => setError(null)}
          >
            {t("label.dismiss")}
          </Button>
        </div>
      ) : null}

      <div
        id="settings-indexer-clients-table-card"
        className="rounded border border-border"
      >
        <div className="flex items-center justify-between gap-3 border-b border-border px-3 py-2">
          <CardTitle className="text-base">
            {t("indexerClients.existing")}
          </CardTitle>
          <IconButton
            id="settings-indexer-clients-refresh"
            label={t("label.refresh")}
            disabled={busy || loading}
            onClick={() => {
              setError(null);
              void load();
            }}
          >
            {loading ? (
              <LoadingMark className="h-4 w-4" />
            ) : (
              <RefreshCw className="h-4 w-4" />
            )}
          </IconButton>
        </div>
        {accessRefusal && clients === null ? (
          <div
            id="settings-indexer-clients-access"
            className="flex items-start gap-2 p-4 text-[var(--scry-muted3)]"
          >
            <Lock className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
            <p>{accessRefusal}</p>
          </div>
        ) : loading && clients === null ? (
          <p className="p-4 text-[var(--scry-muted3)]">{t("label.loading")}</p>
        ) : clients === null ? (
          <p className="p-4 text-[var(--scry-muted3)]">
            {t("status.failedToLoad")}
          </p>
        ) : clients.length === 0 ? (
          <p
            id="settings-indexer-clients-empty"
            className="p-4 text-[var(--scry-muted3)]"
          >
            {t("indexerClients.none")}
          </p>
        ) : (
          <Table
            id="settings-indexer-clients-table"
            overflow="clip"
            layout="fixed"
            density="dense"
            className={NARROW_TABLE}
          >
            <TableHeader>
              <TableRow>
                {sortHead("name", t("label.name"), "w-[22%]")}
                {sortHead(
                  "protocols",
                  t("indexerClients.protocols"),
                  "w-[14%]",
                )}
                {sortHead(
                  "grants",
                  t("indexerClients.grants"),
                  "w-[13%] text-center",
                )}
                {sortHead("enabled", t("label.enabled"), "w-[9%] text-center")}
                {sortHead("key", t("indexerClients.key"), "w-[11%]")}
                {sortHead("lastUsed", t("indexerClients.lastUsed"), "")}
                <TableActionsHead className="w-32">
                  {t("label.actions")}
                </TableActionsHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {(sortedClients ?? []).map((client) => (
                <Fragment key={client.id}>
                  <TableRow
                    id={selectorId("settings-indexer-client-row", client.id)}
                    className={NARROW_ROW}
                    data-ui="settings-table-row"
                    data-client-name={client.name}
                  >
                    <TableCell
                      data-label={t("label.name")}
                      className={cn(
                        "truncate font-medium text-[var(--scry-ink2)]",
                        NARROW_CELL,
                      )}
                    >
                      {client.name}
                    </TableCell>
                    <TableCell
                      data-label={t("indexerClients.protocols")}
                      className={cn("text-[var(--scry-muted3)]", NARROW_CELL)}
                    >
                      {client.protocols
                        .map((protocol) => t(PROTOCOL_LABEL_KEYS[protocol]))
                        .join(" / ")}
                    </TableCell>
                    <TableCell
                      data-label={t("indexerClients.grants")}
                      className={cn(
                        "text-center text-[var(--scry-muted3)]",
                        NARROW_CELL,
                      )}
                    >
                      {client.grants.length}
                    </TableCell>
                    <TableCell
                      data-label={t("label.enabled")}
                      className={cn("text-center", NARROW_CELL)}
                    >
                      <Checkbox
                        id={selectorId(
                          "settings-indexer-client-enabled",
                          client.id,
                        )}
                        size="large"
                        checked={client.enabled}
                        disabled={busy || !client.keyActive}
                        title={
                          client.keyActive
                            ? undefined
                            : t("indexerClients.enableRequiresKey")
                        }
                        aria-label={`${t("label.enabled")}: ${client.name}`}
                        onCheckedChange={(checked) =>
                          toggleEnabled(client, checked === true)
                        }
                      />
                    </TableCell>
                    <TableCell
                      data-label={t("indexerClients.key")}
                      className={NARROW_CELL}
                    >
                      <Badge tone={client.keyActive ? "positive" : "negative"}>
                        {t(
                          client.keyActive
                            ? "indexerClients.keyActive"
                            : "indexerClients.keyRevoked",
                        )}
                      </Badge>
                    </TableCell>
                    <TableCell
                      data-label={t("indexerClients.lastUsed")}
                      className={cn("text-[var(--scry-muted3)]", NARROW_CELL)}
                    >
                      <div className="min-w-0">
                        <div className="truncate">
                          {client.lastUsedAt ? (
                            <time dateTime={client.lastUsedAt}>
                              {formatUiDateTime(
                                client.lastUsedAt,
                                dateTimeFormat,
                              )}
                            </time>
                          ) : (
                            t("indexerClients.never")
                          )}
                        </div>
                        {client.lastOutcome ? (
                          <div
                            className="truncate text-xs"
                            data-client-outcome={client.lastOutcome}
                          >
                            {indexerClientOutcomeMessage(client.lastOutcome, t)}
                          </div>
                        ) : null}
                      </div>
                    </TableCell>
                    <TableActionsCell
                      data-label={t("label.actions")}
                      className={cn(
                        "w-32",
                        NARROW_CELL,
                        "max-[1279px]:w-auto max-[1279px]:border-b-0",
                      )}
                    >
                      <div className="flex items-center justify-end gap-1">
                        <IconButton
                          id={selectorId(
                            "settings-indexer-client-edit",
                            client.id,
                          )}
                          label={t("indexerClients.edit")}
                          tone="edit"
                          disabled={busy}
                          onClick={() => startEdit(client)}
                        >
                          <Edit className="h-4 w-4" />
                        </IconButton>
                        <IconButton
                          id={selectorId(
                            "settings-indexer-client-rotate",
                            client.id,
                          )}
                          label={t("indexerClients.rotate")}
                          tone="upgrade"
                          disabled={busy}
                          onClick={() =>
                            setConfirmation({ client, operation: "rotate" })
                          }
                        >
                          <KeyRound className="h-4 w-4" />
                        </IconButton>
                        <IconButton
                          id={selectorId(
                            "settings-indexer-client-revoke",
                            client.id,
                          )}
                          label={t("indexerClients.revoke")}
                          tone="delete"
                          disabled={busy || !client.keyActive}
                          onClick={() =>
                            setConfirmation({ client, operation: "revoke" })
                          }
                        >
                          <Ban className="h-4 w-4" />
                        </IconButton>
                      </div>
                    </TableActionsCell>
                  </TableRow>
                  {revealed?.clientId === client.id ? (
                    <TableRow
                      id={selectorId(
                        "settings-indexer-client-revealed",
                        client.id,
                      )}
                      className={NARROW_ROW}
                    >
                      {/* A plain cell: the table row hover styles target table-cell slots and would tint the warning. */}
                      <td
                        colSpan={7}
                        className="bg-[var(--scry-warning-bg)] p-3 align-middle max-[1279px]:block"
                      >
                        <div className="flex flex-wrap items-center gap-3">
                          <div className="flex items-center gap-2">
                            <ShieldAlert
                              className="h-4 w-4 shrink-0 text-[var(--scry-warning-text)]"
                              aria-hidden="true"
                            />
                            <p className="font-medium text-[var(--scry-warning-text)]">
                              {t("indexerClients.secretOnce")}
                            </p>
                          </div>
                          <div className="flex w-fit max-w-full items-center">
                            <Input
                              id="indexer-client-revealed-key"
                              aria-label={t("indexerClients.apiKey")}
                              readOnly
                              value={revealed.apiKey}
                              size={revealed.apiKey.length + 2}
                              autoComplete="off"
                              onFocus={(event) => event.currentTarget.select()}
                              className="h-9 w-auto max-w-full rounded-r-none font-[var(--font-code)] text-sm text-[var(--scry-ink2)]"
                            />
                            <IconButton
                              id="indexer-client-revealed-copy"
                              label={
                                revealedCopied
                                  ? t("label.copied")
                                  : t("label.copy")
                              }
                              tone={revealedCopied ? "enabled" : "neutral"}
                              className="-ml-px h-9 shrink-0 rounded-l-none"
                              onClick={() => void copyRevealed()}
                            >
                              {revealedCopied ? (
                                <CheckCircle2 className="h-4 w-4" />
                              ) : (
                                <Clipboard className="h-4 w-4" />
                              )}
                            </IconButton>
                          </div>
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            className="ml-auto"
                            id="indexer-client-revealed-dismiss"
                            onClick={() => setRevealed(null)}
                          >
                            {t("label.dismiss")}
                          </Button>
                        </div>
                      </td>
                    </TableRow>
                  ) : null}
                </Fragment>
              ))}
            </TableBody>
          </Table>
        )}
      </div>

      {clients !== null && editorOpen ? (
        <Card aria-busy={busy}>
          <form
            id="indexer-client-form"
            onSubmit={(event) => {
              event.preventDefault();
              void mutate(
                editing
                  ? updateIndexerClientMutation
                  : createIndexerClientMutation,
                editing
                  ? {
                      id: editing.id,
                      expectedRevision: editing.revision,
                      input: draft,
                    }
                  : { input: draft },
                true,
              );
            }}
          >
            <CardHeader className="flex items-center justify-between gap-3">
              <CardTitle className="text-base">
                {t(editing ? "indexerClients.edit" : "indexerClients.create")}
              </CardTitle>
              <label
                className="flex shrink-0 items-center gap-3"
                title={
                  editing && !editing.keyActive
                    ? t("indexerClients.enableRequiresKey")
                    : undefined
                }
              >
                <Checkbox
                  id="indexer-client-enabled"
                  size="large"
                  checked={draft.enabled}
                  disabled={busy || (editing !== null && !editing.keyActive)}
                  onCheckedChange={(checked) =>
                    setDraft({ ...draft, enabled: checked === true })
                  }
                />
                <span className="text-sm font-medium">
                  {t("label.enabled")}
                </span>
              </label>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="space-y-1.5">
                <Label htmlFor="indexer-client-name">
                  {t("indexerClients.name")}
                </Label>
                <Input
                  id="indexer-client-name"
                  value={draft.name}
                  required
                  maxLength={120}
                  disabled={busy}
                  onChange={(event) =>
                    setDraft({ ...draft, name: event.target.value })
                  }
                />
              </div>
              <fieldset className="space-y-2">
                <legend className="mb-2 text-sm font-medium">
                  {t("indexerClients.protocols")}
                </legend>
                <div className="flex flex-wrap gap-6">
                  {PROTOCOLS.map((protocol) => (
                    <CheckboxField
                      key={protocol}
                      id={`indexer-client-protocol-${protocol.toLowerCase()}`}
                      label={t(PROTOCOL_LABEL_KEYS[protocol])}
                      checked={draft.protocols.includes(protocol)}
                      disabled={busy}
                      onCheckedChange={(checked) =>
                        setDraft({
                          ...draft,
                          protocols:
                            checked === true
                              ? [...draft.protocols, protocol]
                              : draft.protocols.filter(
                                  (value) => value !== protocol,
                                ),
                        })
                      }
                    />
                  ))}
                </div>
              </fieldset>
              <fieldset className="space-y-2">
                <legend className="mb-1 text-sm font-medium">
                  {t("indexerClients.grants")}
                </legend>
                <div className="rounded border border-border">
                  <Table
                    id="indexer-client-grants"
                    overflow="clip"
                    layout="fixed"
                    density="dense"
                    className={NARROW_TABLE}
                  >
                    <TableHeader>
                      <TableRow>
                        <TableHead className="w-12 text-center">
                          <Checkbox
                            id="indexer-client-grant-all"
                            aria-label={t("indexerClients.grantAll")}
                            checked={
                              sources.length > 0 &&
                              draft.grants.length === sources.length
                                ? true
                                : draft.grants.length > 0
                                  ? "indeterminate"
                                  : false
                            }
                            disabled={busy || sources.length === 0}
                            onCheckedChange={(checked) =>
                              setDraft((current) => ({
                                ...current,
                                grants:
                                  checked === true
                                    ? sources.map(
                                        (source) =>
                                          current.grants.find(
                                            (grant) =>
                                              grant.indexerId === source.id,
                                          ) ?? {
                                            indexerId: source.id,
                                            categories: null,
                                          },
                                      )
                                    : [],
                              }))
                            }
                          />
                        </TableHead>
                        <TableHead className="w-[24%]">
                          {t("indexerClients.grantIndexer")}
                        </TableHead>
                        <TableHead className="w-[28%]">
                          {t("settings.baseUrl")}
                        </TableHead>
                        <TableHead className="w-28 text-center">
                          {t("indexerClients.grantAllCategories")}
                        </TableHead>
                        <TableHead>{t("indexerClients.categories")}</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {sources.map((source) => {
                        const grant = draft.grants.find(
                          (value) => value.indexerId === source.id,
                        );
                        return (
                          <TableRow
                            key={source.id}
                            id={selectorId(
                              "indexer-client-grant-row",
                              source.id,
                            )}
                            className={NARROW_ROW}
                            data-ui="settings-table-row"
                          >
                            <TableCell
                              data-label={t("indexerClients.granted")}
                              className={cn("text-center", NARROW_CELL)}
                            >
                              <Checkbox
                                id={`indexer-client-grant-${source.id}`}
                                data-indexer-name={source.name}
                                aria-label={source.name}
                                checked={!!grant}
                                disabled={busy}
                                onCheckedChange={(checked) =>
                                  setGrant(
                                    source.id,
                                    checked === true ? null : undefined,
                                  )
                                }
                              />
                            </TableCell>
                            <TableCell
                              data-label={t("indexerClients.grantIndexer")}
                              className={cn(
                                "truncate font-medium text-[var(--scry-ink2)]",
                                NARROW_CELL,
                              )}
                            >
                              {source.name}
                            </TableCell>
                            <TableCell
                              data-label={t("settings.baseUrl")}
                              className={cn(
                                "truncate text-[var(--scry-muted3)]",
                                NARROW_CELL,
                              )}
                              title={source.baseUrl ?? undefined}
                            >
                              {source.baseUrl}
                            </TableCell>
                            <TableCell
                              data-label={t(
                                "indexerClients.grantAllCategories",
                              )}
                              className={cn(
                                "text-center",
                                NARROW_CELL,
                                !grant && "max-[1279px]:hidden",
                              )}
                            >
                              {grant ? (
                                <Checkbox
                                  id={`indexer-client-all-categories-${source.id}`}
                                  aria-label={`${t("indexerClients.allCategories")}: ${source.name}`}
                                  checked={grant.categories === null}
                                  disabled={busy}
                                  onCheckedChange={(checked) =>
                                    setGrant(
                                      source.id,
                                      checked === true ? null : [],
                                    )
                                  }
                                />
                              ) : null}
                            </TableCell>
                            <TableCell
                              data-label={t("indexerClients.categories")}
                              className={cn(
                                "py-1",
                                NARROW_CELL,
                                "max-[1279px]:border-b-0",
                                !(grant && grant.categories !== null) &&
                                  "max-[1279px]:hidden",
                              )}
                            >
                              {grant && grant.categories !== null ? (
                                <div className="space-y-1">
                                  <IndexerCategoryPicker
                                    scope="RAW"
                                    value={grant.categories}
                                    disabled={busy}
                                    capsCategories={source.capsCategories}
                                    triggerId={`client-categories-${source.id}`}
                                    panelId={`client-categories-panel-${source.id}`}
                                    categoryIdPrefix={`client-category-${source.id}`}
                                    categoriesLabel={t(
                                      "indexerClients.categories",
                                    )}
                                    onChange={(categories) =>
                                      setGrant(source.id, categories)
                                    }
                                  />
                                  {grant.categories.length === 0 ? (
                                    <p className="text-xs text-[var(--scry-warning-text)]">
                                      {t("indexerClients.noCategories")}
                                    </p>
                                  ) : null}
                                </div>
                              ) : null}
                            </TableCell>
                          </TableRow>
                        );
                      })}
                    </TableBody>
                  </Table>
                </div>
              </fieldset>
              <div className="flex flex-wrap gap-2">
                <Button
                  id="indexer-client-save"
                  type="submit"
                  disabled={busy || !draft.name.trim()}
                >
                  {busy ? <LoadingMark /> : null}
                  {t(editing ? "indexerClients.save" : "indexerClients.create")}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  disabled={busy}
                  id="indexer-client-cancel"
                  onClick={closeEditor}
                >
                  {t("label.cancel")}
                </Button>
              </div>
            </CardContent>
          </form>
        </Card>
      ) : null}

      {clients !== null && !(editorOpen && editing === null) ? (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-indexer-client-create"
            icon={Plus}
            label={t("indexerClients.create")}
            onClick={startCreate}
            disabled={busy}
          />
        </div>
      ) : null}

      <ConfirmDialog
        contentId="indexer-client-key-confirmation"
        confirmButtonId="indexer-client-key-confirm"
        cancelButtonId="indexer-client-key-cancel"
        open={confirmation !== null}
        title={
          confirmation
            ? t(
                confirmation.operation === "rotate"
                  ? "indexerClients.rotateConfirmTitle"
                  : "indexerClients.revokeConfirmTitle",
                { name: confirmation.client.name },
              )
            : ""
        }
        description={t("indexerClients.keyChangeWarning")}
        confirmLabel={t(
          confirmation?.operation === "rotate"
            ? "indexerClients.rotate"
            : "indexerClients.revoke",
        )}
        cancelLabel={t("label.cancel")}
        isBusy={busy}
        onCancel={() => setConfirmation(null)}
        onConfirm={() => {
          if (!confirmation) return;
          const { client, operation } = confirmation;
          setConfirmation(null);
          void mutate(
            operation === "rotate"
              ? rotateIndexerClientKeyMutation
              : revokeIndexerClientKeyMutation,
            { id: client.id, expectedRevision: client.revision },
          );
        }}
      />
      <Dialog
        open={mfaOpen}
        onOpenChange={(open) => {
          if (!open) cancelMfa();
        }}
      >
        <DialogContent onInteractOutside={(event) => event.preventDefault()}>
          <DialogHeader>
            <DialogTitle>{t("indexerClients.mfaTitle")}</DialogTitle>
            <DialogDescription>
              {t("indexerClients.mfaDescription")}
            </DialogDescription>
          </DialogHeader>
          <TotpCodeForm
            id="indexer-client-mfa-form"
            inputId="indexer-client-mfa-code"
            code={mfaCode}
            title={t("profile.apiKeyMfaCode")}
            description={t("indexerClients.mfaDescription")}
            submitLabel={t("settings.mfaStepUpSubmit")}
            busyLabel={t("settings.mfaStepUpVerifying")}
            cancelLabel={t("label.cancel")}
            busy={mfaBusy}
            allowRecoveryCode
            onCodeChange={setMfaCode}
            onSubmit={verifyMfa}
            onCancel={cancelMfa}
          />
          {mfaError ? (
            <p role="alert" className="text-sm text-destructive">
              {mfaError}
            </p>
          ) : null}
        </DialogContent>
      </Dialog>
    </div>
  );
}

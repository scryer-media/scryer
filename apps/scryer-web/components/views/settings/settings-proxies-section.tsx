import * as React from "react";
import {
  Copy,
  Edit,
  KeyRound,
  Plus,
  RefreshCw,
  Trash2,
  Upload,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Input,
  integerInputProps,
  sanitizeDigits,
  signedIntegerInputProps,
} from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { formatUiDateTime } from "@/lib/utils/date-format";
import {
  PROXY_FAMILIES,
  PROXY_FAMILY_LABEL_KEYS,
  PROXY_PROVIDER_TYPES_BY_FAMILY,
  WIREGUARD_KEEPALIVE_DEFAULT_SECONDS,
  WIREGUARD_MTU_DEFAULT,
  WIREGUARD_MTU_MAX,
  WIREGUARD_MTU_MIN,
  formatProxyProvider,
  groupProxiesByFamily,
  isProxyProviderType,
  isTunnelProxyProvider,
  isWireguardProxyProvider,
  supportsProxyCredentials,
  supportsProxyHostKey,
  supportsProxyPrivateKey,
  supportsProxyPrivateKeyPassphrase,
  supportsProxyRemoteDns,
  supportsProxyWireguardFields,
} from "@/lib/types";
import type {
  ProxyDraft,
  ProxyProviderTypeValue,
  ProxyRecord,
} from "@/lib/types";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";
import type { BoxedActionButtonTone } from "@/lib/utils/action-button-styles";

export type SettingsProxiesSectionProps = {
  proxyConfigs: ProxyRecord[];
  proxyDraft: ProxyDraft;
  setProxyDraft: React.Dispatch<React.SetStateAction<ProxyDraft>>;
  editingProxyId: string | null;
  isProxyEditorOpen: boolean;
  mutatingProxyId: string | null;
  testingProxyId: string | null;
  resettingHostKeyProxyId: string | null;
  submitProxy: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  resetProxyDraft: () => void;
  startCreateProxy: () => void;
  changeProxyProvider: (providerType: ProxyProviderTypeValue) => void;
  editProxy: (proxy: ProxyRecord) => void;
  /**
   * Fill the draft from a pasted or uploaded configuration. False when the text
   * was not a WireGuard configuration at all, which is when the paste buffer is
   * worth keeping so the operator can see what they gave us.
   */
  importWireguardConfig: (text: string) => boolean;
  testProxy: (proxy: ProxyRecord) => Promise<void> | void;
  deleteProxy: (proxy: ProxyRecord) => Promise<void> | void;
  requestResetHostKey: (proxy: ProxyRecord) => void;
  copyTunnelPublicKey: (publicKey: string) => Promise<void> | void;
};

function ProxyActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<typeof IconButton>, "tone"> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "edit" | "delete" | "search">;
}) {
  return (
    <IconButton label={label} tone={tone} className={className} {...props}>
      {children}
    </IconButton>
  );
}

/// The table's columns, so the empty row and the family headings span the
/// whole width without the two drifting apart.
const PROXY_TABLE_COLUMN_COUNT = 7;

export function SettingsProxiesSection({
  proxyConfigs,
  proxyDraft,
  setProxyDraft,
  editingProxyId,
  isProxyEditorOpen,
  mutatingProxyId,
  testingProxyId,
  resettingHostKeyProxyId,
  submitProxy,
  resetProxyDraft,
  startCreateProxy,
  changeProxyProvider,
  editProxy,
  importWireguardConfig,
  testProxy,
  deleteProxy,
  requestResetHostKey,
  copyTunnelPublicKey,
}: SettingsProxiesSectionProps) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  // The paste buffer is transient: it is what the operator dropped in, not part
  // of the proxy being edited, so it lives here and never reaches the draft.
  const [configText, setConfigText] = React.useState("");
  const configFileRef = React.useRef<HTMLInputElement>(null);

  const applyConfigText = React.useCallback(
    (text: string) => {
      if (importWireguardConfig(text)) {
        setConfigText("");
      }
    },
    [importWireguardConfig],
  );

  const readConfigFile = React.useCallback(
    async (event: React.ChangeEvent<HTMLInputElement>) => {
      const file = event.target.files?.[0];
      // Cleared straight away so choosing the same file twice still fires.
      event.target.value = "";
      if (!file) {
        return;
      }
      applyConfigText(await file.text());
    },
    [applyConfigText],
  );

  const formatProxyHealth = React.useCallback(
    (status: string | null | undefined): string => {
      const normalized = status?.toLowerCase() ?? "";
      if (normalized === "healthy") return t("settings.proxyHealthHealthy");
      if (normalized === "unhealthy") return t("settings.proxyHealthUnhealthy");
      return t("label.unknown");
    },
    [t],
  );

  const groupedProxies = React.useMemo(
    () => groupProxiesByFamily(proxyConfigs),
    [proxyConfigs],
  );

  // Which extra fields the editor may show. Each is narrower than "is not a
  // solver": SOCKS4 carries no credentials, remote DNS is the SOCKS-only
  // `socks4a` / `socks5h` behaviour, and key material is tunnels only.
  const acceptsCredentials = supportsProxyCredentials(proxyDraft.providerType);
  const acceptsRemoteDns = supportsProxyRemoteDns(proxyDraft.providerType);
  const acceptsPrivateKey = supportsProxyPrivateKey(proxyDraft.providerType);
  const isTunnelDraft = isTunnelProxyProvider(proxyDraft.providerType);
  // The tunnel family splits in two: an SSH tunnel has credentials, a key
  // passphrase and a pinned host key; WireGuard has none of those and six
  // fields of its own. The API refuses each half on the other, so the editor
  // shows exactly one of them.
  const isWireguardDraft = isWireguardProxyProvider(proxyDraft.providerType);
  const acceptsWireguardFields = supportsProxyWireguardFields(
    proxyDraft.providerType,
  );
  const acceptsHostKey = supportsProxyHostKey(proxyDraft.providerType);
  // A passphrase only means anything alongside a key, and the API rejects it
  // otherwise, so it appears once one is pasted or already stored — and never
  // for a WireGuard key, which is 32 raw bytes with nothing to unlock.
  const showPassphrase =
    supportsProxyPrivateKeyPassphrase(proxyDraft.providerType) &&
    !proxyDraft.clearPrivateKey &&
    (proxyDraft.privateKey.trim() !== "" || proxyDraft.hasStoredPrivateKey);

  const editingProxy = React.useMemo(
    () =>
      editingProxyId
        ? proxyConfigs.find((proxy) => proxy.id === editingProxyId) ?? null
        : null,
    [editingProxyId, proxyConfigs],
  );

  return (
    <div id="settings-proxies-section" className="flex flex-col gap-4 text-sm">
      <div id="settings-indexer-proxies-panel" className="space-y-4">
        <div id="settings-indexer-proxies-card" className="rounded border border-border">
          <div className="flex items-center justify-between border-b border-border px-3 py-2">
            <CardTitle className="flex items-center gap-2 text-base">
              {t("settings.proxies")}
            </CardTitle>
          </div>
          <p className="border-b border-border px-3 py-2 text-xs text-muted-foreground">
            {t("settings.proxiesHelp")}
          </p>
          <div className="overflow-x-auto">
            <Table id="settings-indexer-proxies-table">
              <TableHeader>
                <TableRow>
                  <TableHead>{t("label.name")}</TableHead>
                  <TableHead>{t("settings.provider")}</TableHead>
                  <TableHead>{t("settings.baseUrl")}</TableHead>
                  <TableHead className="text-center">{t("label.enabled")}</TableHead>
                  <TableHead>{t("settings.proxyHealth")}</TableHead>
                  <TableHead>{t("settings.proxyLastError")}</TableHead>
                  <TableHead className="text-right">{t("label.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {groupedProxies.map((group) => (
                  <React.Fragment key={group.family ?? "other"}>
                    <TableRow
                      id={selectorId(
                        "settings-indexer-proxies-family",
                        group.family ?? "other",
                      )}
                      data-ui="settings-table-group-row"
                    >
                      <TableCell
                        colSpan={PROXY_TABLE_COLUMN_COUNT}
                        className="bg-muted/40 text-xs font-semibold uppercase tracking-wide text-muted-foreground"
                      >
                        {group.family
                          ? t(PROXY_FAMILY_LABEL_KEYS[group.family])
                          : t("settings.proxyFamilyOther")}
                      </TableCell>
                    </TableRow>
                    {group.proxies.map((proxy) => (
                      <TableRow
                        key={proxy.id}
                        id={selectorId("settings-indexer-proxy-row", proxy.name)}
                        data-ui="settings-table-row"
                      >
                        <TableCell className="font-medium">{proxy.name}</TableCell>
                        <TableCell>{formatProxyProvider(proxy.providerType)}</TableCell>
                        <TableCell className="max-w-[280px] truncate">
                          {proxy.baseUrl}
                          {proxy.hasCredentials ? (
                            <span className="ml-2 text-xs text-muted-foreground">
                              {t("settings.proxyCredentialsStored")}
                            </span>
                          ) : null}
                          {proxy.hasPrivateKey ? (
                            <span className="ml-2 text-xs text-muted-foreground">
                              {t("settings.proxyPrivateKeyStored")}
                            </span>
                          ) : null}
                          {proxy.hasPresharedKey ? (
                            <span className="ml-2 text-xs text-muted-foreground">
                              {t("settings.proxyPresharedKeyStored")}
                            </span>
                          ) : null}
                        </TableCell>
                        <TableCell className="text-center">
                          <RenderBooleanIcon
                            value={proxy.isEnabled}
                            label={`${t("label.enabled")}: ${proxy.name}`}
                          />
                        </TableCell>
                        <TableCell>{formatProxyHealth(proxy.lastHealthStatus)}</TableCell>
                        <TableCell>
                          {/* The operator's own date format rather than an
                              English relative phrase, so the column reads the
                              same in every locale. */}
                          {proxy.lastErrorAt ? (
                            <span title={proxy.lastErrorMessage ?? proxy.lastErrorAt}>
                              {formatUiDateTime(proxy.lastErrorAt, dateTimeFormat, {
                                fallback: proxy.lastErrorAt,
                              })}
                            </span>
                          ) : (
                            <span className="text-muted-foreground">—</span>
                          )}
                        </TableCell>
                        <TableCell className="text-right">
                          <div className="flex justify-end gap-2">
                            <ProxyActionButton
                              id={selectorId("settings-indexer-proxy-test", proxy.name)}
                              tone="search"
                              onClick={() => void testProxy(proxy)}
                              disabled={
                                testingProxyId === proxy.id ||
                                mutatingProxyId === proxy.id
                              }
                              label={t("settings.proxyTest")}
                            >
                              <RefreshCw
                                className={cn(
                                  "h-4 w-4",
                                  testingProxyId === proxy.id && "animate-spin",
                                )}
                              />
                            </ProxyActionButton>
                            <ProxyActionButton
                              id={selectorId("settings-indexer-proxy-edit", proxy.name)}
                              tone="edit"
                              onClick={() => editProxy(proxy)}
                              disabled={mutatingProxyId !== null}
                              label={t("label.edit")}
                            >
                              <Edit className="h-4 w-4" />
                            </ProxyActionButton>
                            <ProxyActionButton
                              id={selectorId("settings-indexer-proxy-delete", proxy.name)}
                              tone="delete"
                              onClick={() => void deleteProxy(proxy)}
                              disabled={mutatingProxyId === proxy.id}
                              label={t("label.delete")}
                            >
                              <Trash2 className="h-4 w-4" />
                            </ProxyActionButton>
                          </div>
                        </TableCell>
                      </TableRow>
                    ))}
                  </React.Fragment>
                ))}
                {proxyConfigs.length === 0 ? (
                  <TableRow id="settings-indexer-proxies-empty-row">
                    <TableCell
                      colSpan={PROXY_TABLE_COLUMN_COUNT}
                      className="text-muted-foreground"
                    >
                      {t("settings.proxyEmpty")}
                    </TableCell>
                  </TableRow>
                ) : null}
              </TableBody>
            </Table>
          </div>
        </div>
        {isProxyEditorOpen ? (
          <Card>
            <CardHeader>
              <CardTitle className="text-base">
                {editingProxyId
                  ? t("settings.proxyUpdate")
                  : t("settings.proxyCreateNew")}
              </CardTitle>
            </CardHeader>
            <CardContent>
              <form
                id="settings-indexer-proxy-form"
                className="flex flex-col gap-3"
                onSubmit={submitProxy}
              >
                {/* First, because a whole configuration file is what an
                    operator is handed: filling the form from it beats
                    transcribing eight fields by hand, and every field below
                    still accepts the same lines pasted one at a time. */}
                {isWireguardDraft ? (
                  <div
                    id="settings-indexer-proxy-import-config"
                    className="rounded border border-border bg-card/60 p-3"
                  >
                    <div className="mb-1 text-sm font-medium">
                      {t("settings.proxyImportConfig")}
                    </div>
                    <p className="mb-2 text-xs text-muted-foreground">
                      {t("settings.proxyImportConfigHelp")}
                    </p>
                    <Textarea
                      id="settings-indexer-proxy-import-config-text"
                      className="min-h-24 font-mono text-xs"
                      spellCheck={false}
                      autoComplete="off"
                      rows={5}
                      value={configText}
                      placeholder={
                        "[Interface]\nPrivateKey = …\nAddress = 10.6.0.2/32\n\n[Peer]\nPublicKey = …\nEndpoint = vpn.example.com:51820"
                      }
                      onChange={(event) => setConfigText(event.target.value)}
                    />
                    <input
                      id="settings-indexer-proxy-import-config-file"
                      ref={configFileRef}
                      type="file"
                      accept=".conf,.txt,text/plain"
                      className="hidden"
                      onChange={(event) => {
                        void readConfigFile(event);
                      }}
                    />
                    <div className="mt-2 flex flex-wrap gap-2">
                      <Button
                        id="settings-indexer-proxy-import-config-apply"
                        type="button"
                        variant="outline"
                        disabled={configText.trim() === ""}
                        onClick={() => applyConfigText(configText)}
                      >
                        {t("settings.proxyImportConfigApply")}
                      </Button>
                      <Button
                        id="settings-indexer-proxy-import-config-choose"
                        type="button"
                        variant="outline"
                        onClick={() => configFileRef.current?.click()}
                      >
                        <Upload className="h-4 w-4" />
                        {t("settings.proxyImportConfigFile")}
                      </Button>
                    </div>
                  </div>
                ) : null}
                <div className="grid gap-3 md:grid-cols-[12rem_minmax(0,1fr)_minmax(0,1.4fr)_10rem_auto]">
                  <label>
                    <Label
                      className="mb-2 block"
                      htmlFor="settings-indexer-proxy-provider-type"
                    >
                      {t("settings.provider")}
                    </Label>
                    <Select
                      value={proxyDraft.providerType}
                      disabled={editingProxyId !== null}
                      onValueChange={(value) => {
                        if (!isProxyProviderType(value)) return;
                        changeProxyProvider(value);
                      }}
                    >
                      <SelectTrigger
                        id="settings-indexer-proxy-provider-type"
                        className="w-full"
                      >
                        {/* Rendered explicitly so a provider this client does
                            not know still shows its own value rather than an
                            empty trigger. */}
                        <SelectValue>
                          {formatProxyProvider(proxyDraft.providerType)}
                        </SelectValue>
                      </SelectTrigger>
                      <SelectContent>
                        {PROXY_FAMILIES.map((family) => (
                          <SelectGroup key={family}>
                            <SelectLabel>
                              {t(PROXY_FAMILY_LABEL_KEYS[family])}
                            </SelectLabel>
                            {PROXY_PROVIDER_TYPES_BY_FAMILY[family].map((providerType) => (
                              <SelectItem key={providerType} value={providerType}>
                                {formatProxyProvider(providerType)}
                              </SelectItem>
                            ))}
                          </SelectGroup>
                        ))}
                      </SelectContent>
                    </Select>
                  </label>
                  <label>
                    <Label className="mb-2 block" htmlFor="settings-indexer-proxy-name">
                      {t("label.name")}
                    </Label>
                    <Input
                      id="settings-indexer-proxy-name"
                      value={proxyDraft.name}
                      onChange={(event) =>
                        setProxyDraft((prev) => ({
                          ...prev,
                          name: event.target.value,
                        }))
                      }
                      required
                    />
                  </label>
                  <label>
                    <Label
                      className="mb-2 block"
                      htmlFor="settings-indexer-proxy-base-url"
                    >
                      {isTunnelDraft ? t("settings.proxyEndpoint") : t("settings.baseUrl")}
                    </Label>
                    <Input
                      id="settings-indexer-proxy-base-url"
                      value={proxyDraft.baseUrl}
                      onChange={(event) =>
                        setProxyDraft((prev) => ({
                          ...prev,
                          baseUrl: event.target.value,
                        }))
                      }
                      required
                    />
                  </label>
                  <label>
                    <Label className="mb-2 block" htmlFor="settings-indexer-proxy-timeout">
                      {t("settings.proxyTimeout")}
                    </Label>
                    <Input
                      id="settings-indexer-proxy-timeout"
                      min={1}
                      max={120}
                      {...signedIntegerInputProps}
                      value={proxyDraft.requestTimeoutSeconds}
                      onChange={(event) =>
                        setProxyDraft((prev) => ({
                          ...prev,
                          requestTimeoutSeconds:
                            Number.parseInt(event.target.value, 10) || 1,
                        }))
                      }
                    />
                  </label>
                  <label className="flex items-center gap-2 self-end pb-2">
                    <Checkbox
                      id="settings-indexer-proxy-enabled"
                      checked={proxyDraft.isEnabled}
                      onCheckedChange={(value) =>
                        setProxyDraft((prev) => ({
                          ...prev,
                          isEnabled: value === true,
                        }))
                      }
                    />
                    <span>{t("label.enabled")}</span>
                  </label>
                </div>
                {isTunnelDraft ? (
                  <p
                    id="settings-indexer-proxy-endpoint-help"
                    className="text-xs text-muted-foreground"
                  >
                    {isWireguardDraft
                      ? t("settings.proxyEndpointHelpWireguard")
                      : t("settings.proxyEndpointHelp")}
                  </p>
                ) : null}
                {acceptsCredentials || acceptsRemoteDns ? (
                  <div
                    id="settings-indexer-proxy-transport-fields"
                    className="grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]"
                  >
                    {acceptsCredentials ? (
                      <>
                        <label>
                          <Label
                            className="mb-2 block"
                            htmlFor="settings-indexer-proxy-username"
                          >
                            {t("settings.proxyUsername")}
                          </Label>
                          <Input
                            id="settings-indexer-proxy-username"
                            type={isTunnelDraft ? "text" : "password"}
                            autoComplete="off"
                            value={proxyDraft.username}
                            disabled={proxyDraft.clearCredentials}
                            required={isTunnelDraft && !proxyDraft.hasStoredCredentials}
                            placeholder={
                              proxyDraft.hasStoredCredentials
                                ? t("settings.proxyCredentialUnchanged")
                                : undefined
                            }
                            onChange={(event) =>
                              setProxyDraft((prev) => ({
                                ...prev,
                                username: event.target.value,
                              }))
                            }
                          />
                        </label>
                        <label>
                          <Label
                            className="mb-2 block"
                            htmlFor="settings-indexer-proxy-password"
                          >
                            {t("settings.proxyPassword")}
                          </Label>
                          <Input
                            id="settings-indexer-proxy-password"
                            type="password"
                            autoComplete="new-password"
                            value={proxyDraft.password}
                            disabled={
                              proxyDraft.clearCredentials || proxyDraft.clearPassword
                            }
                            placeholder={
                              proxyDraft.hasStoredCredentials
                                ? t("settings.proxyCredentialUnchanged")
                                : undefined
                            }
                            onChange={(event) =>
                              setProxyDraft((prev) => ({
                                ...prev,
                                password: event.target.value,
                              }))
                            }
                          />
                        </label>
                      </>
                    ) : null}
                    <div className="flex flex-col justify-end gap-2 pb-2">
                      {acceptsRemoteDns ? (
                        <label className="flex items-center gap-2">
                          <Checkbox
                            id="settings-indexer-proxy-remote-dns"
                            checked={proxyDraft.remoteDns}
                            onCheckedChange={(value) =>
                              setProxyDraft((prev) => ({
                                ...prev,
                                remoteDns: value === true,
                              }))
                            }
                          />
                          <span>{t("settings.proxyRemoteDns")}</span>
                        </label>
                      ) : null}
                      {acceptsCredentials &&
                      !isTunnelDraft &&
                      proxyDraft.hasStoredCredentials ? (
                        <label className="flex items-center gap-2">
                          <Checkbox
                            id="settings-indexer-proxy-clear-credentials"
                            checked={proxyDraft.clearCredentials}
                            onCheckedChange={(value) =>
                              setProxyDraft((prev) => ({
                                ...prev,
                                clearCredentials: value === true,
                                username: "",
                                password: "",
                              }))
                            }
                          />
                          <span>{t("settings.proxyClearCredentials")}</span>
                        </label>
                      ) : null}
                      {isTunnelDraft && proxyDraft.hasStoredCredentials ? (
                        <label className="flex items-center gap-2">
                          <Checkbox
                            id="settings-indexer-proxy-clear-password"
                            checked={proxyDraft.clearPassword}
                            onCheckedChange={(value) =>
                              setProxyDraft((prev) => ({
                                ...prev,
                                clearPassword: value === true,
                                password: "",
                              }))
                            }
                          />
                          <span>{t("settings.proxyClearPassword")}</span>
                        </label>
                      ) : null}
                    </div>
                    <p className="text-xs text-muted-foreground md:col-span-3">
                      {isTunnelDraft ? t("settings.proxyTunnelAuthHelp") : null}
                      {isTunnelDraft && acceptsCredentials ? " " : null}
                      {acceptsCredentials
                        ? proxyDraft.hasStoredCredentials
                          ? t("settings.proxyCredentialsStoredHelp")
                          : t("settings.proxyCredentialsHelp")
                        : null}
                      {acceptsCredentials && acceptsRemoteDns ? " " : null}
                      {acceptsRemoteDns ? t("settings.proxyRemoteDnsHelp") : null}
                    </p>
                  </div>
                ) : null}
                {acceptsPrivateKey ? (
                  <div
                    id="settings-indexer-proxy-tunnel-fields"
                    className="flex flex-col gap-3"
                  >
                    <label>
                      <Label
                        className="mb-2 block"
                        htmlFor="settings-indexer-proxy-private-key"
                      >
                        {t("settings.proxyPrivateKey")}
                      </Label>
                      {/* A WireGuard key is one 44-character base64 line, not
                          a PEM block, so it gets a single-line field. Neither
                          is ever read back, so neither is a password input:
                          masking a value the operator is pasting in helps
                          nobody. */}
                      {isWireguardDraft ? (
                        <Input
                          id="settings-indexer-proxy-private-key"
                          className="font-mono text-xs"
                          type="text"
                          spellCheck={false}
                          autoComplete="off"
                          ignorePasswordManagers
                          value={proxyDraft.privateKey}
                          disabled={proxyDraft.clearPrivateKey}
                          required={
                            !proxyDraft.hasStoredPrivateKey ||
                            proxyDraft.clearPrivateKey
                          }
                          placeholder={
                            proxyDraft.hasStoredPrivateKey
                              ? t("settings.proxyPrivateKeyUnchanged")
                              : "PrivateKey = …"
                          }
                          onChange={(event) =>
                            setProxyDraft((prev) => ({
                              ...prev,
                              privateKey: event.target.value,
                            }))
                          }
                        />
                      ) : (
                        <Textarea
                          id="settings-indexer-proxy-private-key"
                          className="min-h-32 font-mono text-xs"
                          spellCheck={false}
                          autoComplete="off"
                          rows={8}
                          value={proxyDraft.privateKey}
                          disabled={proxyDraft.clearPrivateKey}
                          placeholder={
                            proxyDraft.hasStoredPrivateKey
                              ? t("settings.proxyPrivateKeyUnchanged")
                              : "-----BEGIN OPENSSH PRIVATE KEY-----"
                          }
                          onChange={(event) =>
                            setProxyDraft((prev) => ({
                              ...prev,
                              privateKey: event.target.value,
                            }))
                          }
                        />
                      )}
                    </label>
                    <p
                      id="settings-indexer-proxy-private-key-help"
                      className="text-xs text-muted-foreground"
                    >
                      {/* The backend's own sentence, so the help text and the
                          save-time rejection cannot say different things. */}
                      {isWireguardDraft
                        ? t("settings.proxyPrivateKeyHelpWireguard")
                        : t("settings.proxyPrivateKeyHelp")}
                    </p>
                    {/* A WireGuard tunnel cannot exist without its key, so
                        "clear it" is a state the API refuses; the toggle is
                        withheld the same way a tunnel's mandatory username
                        is. To rotate the key, paste a new one. */}
                    {proxyDraft.hasStoredPrivateKey && !isWireguardDraft ? (
                      <label className="flex items-center gap-2">
                        <Checkbox
                          id="settings-indexer-proxy-clear-private-key"
                          checked={proxyDraft.clearPrivateKey}
                          onCheckedChange={(value) =>
                            setProxyDraft((prev) => ({
                              ...prev,
                              clearPrivateKey: value === true,
                              privateKey: "",
                              privateKeyPassphrase: "",
                            }))
                          }
                        />
                        <span>{t("settings.proxyClearPrivateKey")}</span>
                      </label>
                    ) : null}
                    {showPassphrase ? (
                      <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                        <label>
                          <Label
                            className="mb-2 block"
                            htmlFor="settings-indexer-proxy-private-key-passphrase"
                          >
                            {t("settings.proxyPrivateKeyPassphrase")}
                          </Label>
                          <Input
                            id="settings-indexer-proxy-private-key-passphrase"
                            type="password"
                            autoComplete="new-password"
                            value={proxyDraft.privateKeyPassphrase}
                            placeholder={
                              proxyDraft.hasStoredPrivateKey
                                ? t("settings.proxyCredentialUnchanged")
                                : undefined
                            }
                            onChange={(event) =>
                              setProxyDraft((prev) => ({
                                ...prev,
                                privateKeyPassphrase: event.target.value,
                              }))
                            }
                          />
                        </label>
                        <p className="self-end pb-2 text-xs text-muted-foreground">
                          {t("settings.proxyPrivateKeyPassphraseHelp")}
                        </p>
                      </div>
                    ) : null}
                    {acceptsWireguardFields ? (
                      <>
                        <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                          <label>
                            <Label
                              className="mb-2 block"
                              htmlFor="settings-indexer-proxy-peer-public-key"
                            >
                              {t("settings.proxyPeerPublicKey")}
                            </Label>
                            {/* A public key is public: it is read back in full
                                and shown as typed, never masked. */}
                            <Input
                              id="settings-indexer-proxy-peer-public-key"
                              className="font-mono text-xs"
                              type="text"
                              spellCheck={false}
                              autoComplete="off"
                              ignorePasswordManagers
                              required
                              value={proxyDraft.peerPublicKey}
                              placeholder="PublicKey = …"
                              onChange={(event) =>
                                setProxyDraft((prev) => ({
                                  ...prev,
                                  peerPublicKey: event.target.value,
                                }))
                              }
                            />
                            <p className="mt-1 text-xs text-muted-foreground">
                              {t("settings.proxyPeerPublicKeyHelp")}
                            </p>
                          </label>
                          {/* A div rather than a wrapping label: the clear
                              toggle sits in this cell, and a label around both
                              would send its clicks to the text field. */}
                          <div>
                            <Label
                              className="mb-2 block"
                              htmlFor="settings-indexer-proxy-preshared-key"
                            >
                              {t("settings.proxyPresharedKey")}
                            </Label>
                            <Input
                              id="settings-indexer-proxy-preshared-key"
                              className="font-mono text-xs"
                              type="text"
                              spellCheck={false}
                              autoComplete="off"
                              ignorePasswordManagers
                              value={proxyDraft.presharedKey}
                              disabled={proxyDraft.clearPresharedKey}
                              placeholder={
                                proxyDraft.hasStoredPresharedKey
                                  ? t("settings.proxyPresharedKeyUnchanged")
                                  : "PresharedKey = …"
                              }
                              onChange={(event) =>
                                setProxyDraft((prev) => ({
                                  ...prev,
                                  presharedKey: event.target.value,
                                }))
                              }
                            />
                            <p className="mt-1 text-xs text-muted-foreground">
                              {t("settings.proxyPresharedKeyHelp")}
                            </p>
                            {proxyDraft.hasStoredPresharedKey ? (
                              <label className="mt-2 flex items-center gap-2">
                                <Checkbox
                                  id="settings-indexer-proxy-clear-preshared-key"
                                  checked={proxyDraft.clearPresharedKey}
                                  onCheckedChange={(value) =>
                                    setProxyDraft((prev) => ({
                                      ...prev,
                                      clearPresharedKey: value === true,
                                      presharedKey: "",
                                    }))
                                  }
                                />
                                <span>{t("settings.proxyClearPresharedKey")}</span>
                              </label>
                            ) : null}
                          </div>
                        </div>
                        <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                          <label>
                            <Label
                              className="mb-2 block"
                              htmlFor="settings-indexer-proxy-tunnel-addresses"
                            >
                              {t("settings.proxyTunnelAddresses")}
                            </Label>
                            <Textarea
                              id="settings-indexer-proxy-tunnel-addresses"
                              className="min-h-20 font-mono text-xs"
                              spellCheck={false}
                              rows={3}
                              required
                              value={proxyDraft.tunnelAddresses}
                              placeholder="10.6.0.2/32"
                              onChange={(event) =>
                                setProxyDraft((prev) => ({
                                  ...prev,
                                  tunnelAddresses: event.target.value,
                                }))
                              }
                            />
                            <p className="mt-1 text-xs text-muted-foreground">
                              {t("settings.proxyTunnelAddressesHelp")}
                            </p>
                          </label>
                          <label>
                            <Label
                              className="mb-2 block"
                              htmlFor="settings-indexer-proxy-tunnel-dns-servers"
                            >
                              {t("settings.proxyTunnelDnsServers")}
                            </Label>
                            <Textarea
                              id="settings-indexer-proxy-tunnel-dns-servers"
                              className="min-h-20 font-mono text-xs"
                              spellCheck={false}
                              rows={3}
                              value={proxyDraft.tunnelDnsServers}
                              placeholder="10.6.0.1"
                              onChange={(event) =>
                                setProxyDraft((prev) => ({
                                  ...prev,
                                  tunnelDnsServers: event.target.value,
                                }))
                              }
                            />
                            <p className="mt-1 text-xs text-muted-foreground">
                              {t("settings.proxyTunnelDnsServersHelp")}
                            </p>
                          </label>
                        </div>
                        <div className="grid gap-3 md:grid-cols-[10rem_10rem_minmax(0,1fr)]">
                          <label>
                            <Label
                              className="mb-2 block"
                              htmlFor="settings-indexer-proxy-tunnel-mtu"
                            >
                              {t("settings.proxyTunnelMtu")}
                            </Label>
                            {/* Blank is a real value here — it means "use the
                                engine's default" — so these are text fields
                                that keep an empty string rather than number
                                inputs that coerce one. */}
                            <Input
                              id="settings-indexer-proxy-tunnel-mtu"
                              {...integerInputProps}
                              value={proxyDraft.tunnelMtu}
                              placeholder={String(WIREGUARD_MTU_DEFAULT)}
                              onChange={(event) =>
                                setProxyDraft((prev) => ({
                                  ...prev,
                                  tunnelMtu: sanitizeDigits(event.target.value),
                                }))
                              }
                            />
                          </label>
                          <label>
                            <Label
                              className="mb-2 block"
                              htmlFor="settings-indexer-proxy-tunnel-keepalive"
                            >
                              {t("settings.proxyTunnelKeepalive")}
                            </Label>
                            <Input
                              id="settings-indexer-proxy-tunnel-keepalive"
                              {...integerInputProps}
                              value={proxyDraft.tunnelKeepaliveSeconds}
                              placeholder={String(
                                WIREGUARD_KEEPALIVE_DEFAULT_SECONDS,
                              )}
                              onChange={(event) =>
                                setProxyDraft((prev) => ({
                                  ...prev,
                                  tunnelKeepaliveSeconds: sanitizeDigits(
                                    event.target.value,
                                  ),
                                }))
                              }
                            />
                          </label>
                          <div className="flex flex-col justify-end gap-1 pb-2 text-xs text-muted-foreground">
                            <span>
                              {t("settings.proxyTunnelMtuHelp", {
                                min: WIREGUARD_MTU_MIN,
                                max: WIREGUARD_MTU_MAX,
                                default: WIREGUARD_MTU_DEFAULT,
                              })}
                            </span>
                            <span>
                              {t("settings.proxyTunnelKeepaliveHelp", {
                                default: WIREGUARD_KEEPALIVE_DEFAULT_SECONDS,
                              })}
                            </span>
                          </div>
                        </div>
                        {/* The one value the operator has to carry back to
                            their server, so it is shown in full with a copy
                            button rather than left to a select-and-drag. */}
                        <div
                          id="settings-indexer-proxy-tunnel-public-key"
                          className="rounded border border-border bg-card/60 p-3"
                        >
                          <div className="mb-1 text-sm font-medium">
                            {t("settings.proxyTunnelPublicKey")}
                          </div>
                          {editingProxy?.tunnelPublicKey ? (
                            <>
                              <div className="break-all font-mono text-xs">
                                {editingProxy.tunnelPublicKey}
                              </div>
                              <p className="mt-1 text-xs text-muted-foreground">
                                {t("settings.proxyTunnelPublicKeyHelp")}
                              </p>
                              <Button
                                id="settings-indexer-proxy-tunnel-public-key-copy"
                                type="button"
                                variant="outline"
                                className="mt-2"
                                onClick={() =>
                                  void copyTunnelPublicKey(
                                    editingProxy.tunnelPublicKey ?? "",
                                  )
                                }
                              >
                                <Copy className="h-4 w-4" />
                                {t("settings.proxyTunnelPublicKeyCopy")}
                              </Button>
                            </>
                          ) : (
                            <p className="text-xs text-muted-foreground">
                              {t("settings.proxyTunnelPublicKeyPending")}
                            </p>
                          )}
                        </div>
                      </>
                    ) : null}
                    {acceptsHostKey && editingProxy ? (
                      <div
                        id="settings-indexer-proxy-host-key"
                        className="rounded border border-border bg-card/60 p-3"
                      >
                        <div className="mb-1 text-sm font-medium">
                          {t("settings.proxyHostKey")}
                        </div>
                        {editingProxy.hostKeyFingerprint ? (
                          <>
                            <div
                              id="settings-indexer-proxy-host-key-fingerprint"
                              className="break-all font-mono text-xs"
                            >
                              {editingProxy.hostKeyFingerprint}
                            </div>
                            <p className="mt-1 text-xs text-muted-foreground">
                              {t("settings.proxyHostKeyPinnedAt", {
                                time: formatUiDateTime(
                                  editingProxy.hostKeyPinnedAt,
                                  dateTimeFormat,
                                  { fallback: "—" },
                                ),
                              })}
                            </p>
                            <Button
                              id="settings-indexer-proxy-host-key-reset"
                              type="button"
                              variant="outline"
                              className="mt-2"
                              onClick={() => requestResetHostKey(editingProxy)}
                              disabled={
                                resettingHostKeyProxyId === editingProxy.id ||
                                mutatingProxyId !== null
                              }
                            >
                              <KeyRound className="h-4 w-4" />
                              {t("settings.proxyHostKeyReset")}
                            </Button>
                          </>
                        ) : (
                          <p
                            id="settings-indexer-proxy-host-key-unpinned"
                            className="text-xs text-muted-foreground"
                          >
                            {t("settings.proxyHostKeyUnpinned")}
                          </p>
                        )}
                      </div>
                    ) : null}
                  </div>
                ) : null}
                <div className="flex items-end gap-2">
                  <Button
                    id="settings-indexer-proxy-save"
                    type="submit"
                    disabled={mutatingProxyId !== null}
                  >
                    {mutatingProxyId
                      ? t("label.saving")
                      : editingProxyId
                        ? t("settings.proxyUpdate")
                        : t("settings.proxyCreate")}
                  </Button>
                  {editingProxyId ? (
                    <Button
                      id="settings-indexer-proxy-cancel"
                      type="button"
                      variant="outline"
                      onClick={resetProxyDraft}
                      disabled={mutatingProxyId !== null}
                    >
                      {t("label.cancel")}
                    </Button>
                  ) : null}
                </div>
              </form>
            </CardContent>
          </Card>
        ) : (
          <div className="flex justify-center">
            <AddNewButton
              id="settings-indexer-proxy-create"
              icon={Plus}
              label={t("settings.proxyCreateNew")}
              onClick={startCreateProxy}
              disabled={mutatingProxyId !== null}
            />
          </div>
        )}
      </div>
    </div>
  );
}

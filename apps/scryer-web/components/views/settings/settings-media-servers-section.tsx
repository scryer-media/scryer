import * as React from "react";
import { Link } from "react-router";
import {
  CheckCircle2,
  Edit,
  KeyRound,
  Loader2,
  Plus,
  Power,
  PowerOff,
  Server,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { LocalRemotePathMappingsField } from "@/components/common/local-remote-path-mappings-field";
import {
  PermissionDropdowns,
  type LibraryPermissionDrafts,
} from "@/components/common/permission-checkboxes";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  isVisibleMediaServerProvider,
  type VisibleMediaServerProvider,
} from "@/lib/constants/integration-providers";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  LibraryRecord,
  MediaServerConnection,
  MediaServerConnectionDraft,
  MediaServerDefaultLibraryGrant,
  MediaServerProvider,
  PlexServerDiscovery,
} from "@/lib/types";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";
import type { BoxedActionButtonTone } from "@/lib/utils/action-button-styles";
import type { LocalPathStyle } from "@/lib/utils/local-path-style";
import { buildViewPath } from "@/lib/utils/routing";

type SettingsMediaServersSectionProps = {
  connections: MediaServerConnection[];
  libraries: LibraryRecord[];
  draft: MediaServerConnectionDraft;
  setDraft: React.Dispatch<React.SetStateAction<MediaServerConnectionDraft>>;
  localPathStyle: LocalPathStyle | undefined;
  pathMappingsValid: boolean;
  onPathMappingsValidityChange: (isValid: boolean) => void;
  effectiveFormLoginEnabled: boolean;
  editingConnectionId: string | null;
  mutatingConnectionId: string | null;
  testingConnectionId: string | null;
  isEditorOpen: boolean;
  editorMode: "create" | "edit";
  submitConnection: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  editConnection: (connection: MediaServerConnection) => void;
  testConnection: (connection: MediaServerConnection) => Promise<void> | void;
  toggleConnectionEnabled: (connection: MediaServerConnection) => Promise<void> | void;
  deleteConnection: (connection: MediaServerConnection) => Promise<void> | void;
  resetDraft: () => void;
  startCreateConnection: () => void;
  plexServerOptions: PlexServerDiscovery[];
  plexDiscoveryBusy: boolean;
  discoverPlexServers: () => Promise<void> | void;
  embyConnectBusy: boolean;
  discoverEmbyConnectServers: () => Promise<void> | void;
  testSavedEmbyConnect: () => Promise<void> | void;
  editorError: string | null;
};

const PROVIDERS: Array<{ value: VisibleMediaServerProvider; label: string }> = [
  { value: "JELLYFIN", label: "Jellyfin" },
  { value: "PLEX", label: "Plex" },
  { value: "EMBY", label: "Emby" },
];

const DEFAULT_BASE_URL_BY_PROVIDER: Record<MediaServerProvider, string> = {
  JELLYFIN: "",
  PLEX: "https://plex.tv",
  EMBY: "",
};

const DEFAULT_NAME_BY_PROVIDER: Record<MediaServerProvider, string> = {
  JELLYFIN: "Jellyfin",
  PLEX: "Plex",
  EMBY: "Emby",
};

function providerLabel(provider: MediaServerProvider): string {
  return PROVIDERS.find((candidate) => candidate.value === provider)?.label ?? provider;
}

function MediaServerProviderLogo({ provider }: { provider: MediaServerProvider }) {
  return (
    <img
      src={`/auth-providers/${provider.toLowerCase()}.svg`}
      alt=""
      aria-hidden="true"
      className="h-5 w-5 shrink-0 object-contain"
    />
  );
}

function providerSupportsAuth(provider: MediaServerProvider): boolean {
  return provider === "JELLYFIN" || provider === "PLEX" || provider === "EMBY";
}

function updateLibraryGrant(
  grants: MediaServerDefaultLibraryGrant[],
  libraryId: string,
  permissions: string[],
): MediaServerDefaultLibraryGrant[] {
  const filtered = grants.filter((grant) => grant.libraryId !== libraryId);
  const normalized = Array.from(new Set(permissions.map((permission) => permission.trim()).filter(Boolean)));
  return normalized.length > 0
    ? [...filtered, { libraryId, permissions: normalized }]
    : filtered;
}

type CapabilityBadgeTone = "neutral" | "positive" | "info";

function capabilityBadges(
  connection: MediaServerConnection,
  effectiveFormLoginEnabled: boolean,
): Array<{ label: string; tone: CapabilityBadgeTone }> {
  const badges: Array<{ label: string; tone: CapabilityBadgeTone }> = [];
  if (effectiveFormLoginEnabled && connection.loginEnabled) {
    badges.push({ label: "Login", tone: "positive" });
  }
  if (connection.linkingEnabled) {
    badges.push({ label: "Linking", tone: "info" });
  }
  if (connection.autoAddEnabled) {
    badges.push({ label: "Auto-add", tone: "info" });
  }
  return badges.length > 0
    ? badges
    : [{ label: "Connection only", tone: "neutral" }];
}

function MediaServerActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<typeof IconButton>, "tone"> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "edit" | "enabled" | "disabled" | "delete" | "neutral">;
}) {
  return (
    <IconButton label={label} tone={tone} className={className} {...props}>
      {children}
    </IconButton>
  );
}

export function SettingsMediaServersSection({
  connections,
  libraries,
  draft,
  setDraft,
  localPathStyle,
  pathMappingsValid,
  onPathMappingsValidityChange,
  effectiveFormLoginEnabled,
  editingConnectionId,
  mutatingConnectionId,
  testingConnectionId,
  isEditorOpen,
  editorMode,
  submitConnection,
  editConnection,
  testConnection,
  toggleConnectionEnabled,
  deleteConnection,
  resetDraft,
  startCreateConnection,
  plexServerOptions,
  plexDiscoveryBusy,
  discoverPlexServers,
  embyConnectBusy,
  discoverEmbyConnectServers,
  testSavedEmbyConnect,
  editorError,
}: SettingsMediaServersSectionProps) {
  const t = useTranslate();
  const isEditing = editorMode === "edit";
  const supportsAuth = providerSupportsAuth(draft.provider);
  const selectedProviderLabel = providerLabel(draft.provider);
  const editorMutationId = editingConnectionId ?? "new";
  const isSavingEditor = mutatingConnectionId === editorMutationId;
  const formLoginSettingsPath = buildViewPath("settings", "security");
  const visibleConnections = connections.filter((connection) =>
    isVisibleMediaServerProvider(connection.provider),
  );

  const handleProviderChange = React.useCallback(
    (provider: MediaServerProvider) => {
      setDraft((previous) => {
        const wasAutofilledName =
          previous.displayName.trim().length === 0 ||
          previous.displayName === DEFAULT_NAME_BY_PROVIDER[previous.provider];
        return {
          ...previous,
          provider,
          displayName: wasAutofilledName
            ? DEFAULT_NAME_BY_PROVIDER[provider]
            : previous.displayName,
          baseUrl: previous.baseUrl.trim() || DEFAULT_BASE_URL_BY_PROVIDER[provider],
          loginEnabled: providerSupportsAuth(provider) ? previous.loginEnabled : false,
          linkingEnabled: providerSupportsAuth(provider) ? previous.linkingEnabled : false,
          autoAddEnabled: providerSupportsAuth(provider) ? previous.autoAddEnabled : false,
          defaultAppPermissions: providerSupportsAuth(provider)
            ? previous.defaultAppPermissions
            : [],
          defaultLibraryGrants: providerSupportsAuth(provider)
            ? previous.defaultLibraryGrants
            : [],
          machineIdPresent: provider === "PLEX" ? previous.machineIdPresent : false,
          plexServerId: provider === "PLEX" ? previous.plexServerId : "",
          jellyfinCredentialMode:
            provider === "JELLYFIN" ? "adminLogin" : previous.jellyfinCredentialMode,
          embyConnectionMode: provider === "EMBY" ? "LOCAL" : previous.embyConnectionMode,
          embyLocalSetupMethod:
            provider === "EMBY" ? "API_KEY" : previous.embyLocalSetupMethod,
          embyConnectEnabled: provider === "EMBY" ? false : previous.embyConnectEnabled,
          embyConnectUsernameOrEmail: "",
          embyConnectPassword: "",
          embyConnectServerId: "",
          embyDiscoveredServers: [],
          apiKey: provider === "JELLYFIN" || provider === "EMBY" ? "" : previous.apiKey,
          clearApiKey:
            provider === "JELLYFIN" || provider === "EMBY" ? false : previous.clearApiKey,
        };
      });
    },
    [setDraft],
  );

  return (
    <div id="settings-media-servers-section" className="space-y-4 text-sm">
      <div id="settings-media-servers-table-card" className="rounded border border-border">
        <div className="overflow-x-auto">
          <Table id="settings-media-servers-table">
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead>{t("settings.provider")}</TableHead>
                <TableHead>{t("settings.baseUrl")}</TableHead>
                <TableHead>{t("label.enabled")}</TableHead>
                <TableHead>{t("settings.capabilities")}</TableHead>
                <TableHead>{t("settings.credentials")}</TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {visibleConnections.map((connection) => (
                <TableRow
                  data-ui="settings-table-row"
                  key={connection.id}
                  id={selectorId("settings-media-server-row", connection.id)}
                >
                  <TableCell className="font-medium">{connection.displayName}</TableCell>
                  <TableCell>
                    <span className="flex items-center gap-2">
                      <MediaServerProviderLogo provider={connection.provider} />
                      {providerLabel(connection.provider)}
                    </span>
                  </TableCell>
                  <TableCell className="max-w-72 truncate">{connection.baseUrl || "-"}</TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={connection.enabled}
                      label={`${t("label.enabled")}: ${connection.displayName}`}
                    />
                  </TableCell>
                  <TableCell>
                    <div className="flex flex-wrap gap-1.5">
                      {capabilityBadges(connection, effectiveFormLoginEnabled).map((badge) => (
                        <Badge key={badge.label} tone={badge.tone} className="rounded-full">
                          {badge.label}
                        </Badge>
                      ))}
                    </div>
                  </TableCell>
                  <TableCell>
                    {connection.provider === "PLEX" ? (
                      connection.machineIdPresent ? (
                        <span className="inline-flex items-center gap-1 text-[var(--scry-success-text)]">
                          <CheckCircle2 className="h-3.5 w-3.5" />
                          {t("settings.plexServerSelected")}
                        </span>
                      ) : (
                        <span className="text-muted-foreground">{t("settings.plexServerMissing")}</span>
                      )
                    ) : connection.provider === "EMBY" ? (
                      <div className="space-y-1">
                        <span
                          id="settings-emby-api-key-present"
                          className={cn(
                            "inline-flex items-center gap-1",
                            connection.apiKeyPresent
                              ? "text-[var(--scry-success-text)]"
                              : "text-muted-foreground",
                          )}
                        >
                          <KeyRound className="h-3.5 w-3.5" />
                          {connection.apiKeyPresent
                            ? t("settings.apiKeyConfigured")
                            : t("settings.apiKeyMissing")}
                        </span>
                        <span
                          id="settings-emby-server-id-present"
                          className={cn(
                            "block",
                            connection.embyServerIdPresent
                              ? "text-[var(--scry-success-text)]"
                              : "text-muted-foreground",
                          )}
                        >
                          {connection.embyServerIdPresent
                            ? "Server identity verified"
                            : "Server identity missing"}
                        </span>
                      </div>
                    ) : connection.apiKeyPresent ? (
                      <span className="inline-flex items-center gap-1 text-[var(--scry-success-text)]">
                        <KeyRound className="h-3.5 w-3.5" />
                        {t("settings.apiKeyConfigured")}
                      </span>
                    ) : (
                      <span className="text-muted-foreground">{t("settings.apiKeyMissing")}</span>
                    )}
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="inline-flex items-center gap-2">
                      <MediaServerActionButton
                        id={selectorId("settings-media-server-test", connection.id)}
                        label={t("label.testConnection")}
                        tone="neutral"
                        onClick={() => void testConnection(connection)}
                        disabled={testingConnectionId === connection.id}
                      >
                        {testingConnectionId === connection.id ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <ShieldCheck className="h-4 w-4" />
                        )}
                      </MediaServerActionButton>
                      <MediaServerActionButton
                        id={selectorId("settings-media-server-toggle", connection.id)}
                        label={connection.enabled ? t("label.disable") : t("label.enable")}
                        tone={connection.enabled ? "enabled" : "disabled"}
                        onClick={() => void toggleConnectionEnabled(connection)}
                        disabled={mutatingConnectionId === connection.id}
                      >
                        {connection.enabled ? (
                          <Power className="h-4 w-4" />
                        ) : (
                          <PowerOff className="h-4 w-4" />
                        )}
                      </MediaServerActionButton>
                      <MediaServerActionButton
                        id={selectorId("settings-media-server-edit", connection.id)}
                        label={t("label.edit")}
                        tone="edit"
                        onClick={() => editConnection(connection)}
                      >
                        <Edit className="h-4 w-4" />
                      </MediaServerActionButton>
                      <MediaServerActionButton
                        id={selectorId("settings-media-server-delete", connection.id)}
                        label={t("label.delete")}
                        tone="delete"
                        onClick={() => void deleteConnection(connection)}
                        disabled={mutatingConnectionId === connection.id}
                      >
                        {mutatingConnectionId === connection.id ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <Trash2 className="h-4 w-4" />
                        )}
                      </MediaServerActionButton>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
              {visibleConnections.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-muted-foreground">
                    {t("settings.noMediaServersFound")}
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </div>

      {isEditorOpen ? (
        <>
          <Card>
            <CardHeader>
              <CardTitle id="settings-media-server-editor" className="text-base">
                {isEditing
                  ? t("settings.mediaServerUpdate")
                  : t("settings.mediaServerCreate")}
              </CardTitle>
            </CardHeader>
            <CardContent>
              <form
                id="settings-media-server-form"
                className="space-y-4"
                onSubmit={submitConnection}
              >
                <div className="grid gap-3 md:grid-cols-3">
                  <label>
                    <Label className="mb-2 block" htmlFor="settings-media-server-provider">
                      {t("settings.provider")}
                    </Label>
                    <Select
                      value={draft.provider}
                      onValueChange={(value) => handleProviderChange(value as MediaServerProvider)}
                    >
                      <SelectTrigger id="settings-media-server-provider" className="w-full">
                        <SelectValue aria-label={selectedProviderLabel} />
                      </SelectTrigger>
                      <SelectContent>
                        {PROVIDERS.map((provider) => (
                          <SelectItem
                            id={selectorId("settings-media-server-provider", provider.value)}
                            key={provider.value}
                            value={provider.value}
                          >
                            <span className="flex items-center gap-2">
                              <MediaServerProviderLogo provider={provider.value} />
                              {provider.label}
                            </span>
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </label>
                  <label>
                    <Label className="mb-2 block" htmlFor="settings-media-server-name">
                      {t("label.name")}
                    </Label>
                    <Input
                      id="settings-media-server-name"
                      value={draft.displayName}
                      onChange={(event) =>
                        setDraft((previous) => ({
                          ...previous,
                          displayName: event.target.value,
                        }))
                      }
                      required
                      placeholder={selectedProviderLabel}
                    />
                  </label>
                  <label>
                    <Label
                      className="mb-2 block"
                      htmlFor={
                        draft.provider === "EMBY" && draft.embyConnectionMode === "CONNECT"
                          ? "settings-emby-connect-base-url"
                          : "settings-media-server-base-url"
                      }
                    >
                      {t("settings.baseUrl")}
                    </Label>
                    <Input
                      id={
                        draft.provider === "EMBY" && draft.embyConnectionMode === "CONNECT"
                          ? "settings-emby-connect-base-url"
                          : "settings-media-server-base-url"
                      }
                      value={draft.baseUrl}
                      onChange={(event) =>
                        setDraft((previous) => ({
                          ...previous,
                          baseUrl: event.target.value,
                        }))
                      }
                      required={draft.provider !== "PLEX"}
                      placeholder={
                        draft.provider === "PLEX"
                          ? "https://plex.tv"
                          : `https://${draft.provider}.example.test`
                      }
                    />
                  </label>
                </div>

                {draft.provider !== "PLEX" ? (
                  <label className="block">
                    <Label className="mb-2 block" htmlFor="settings-media-server-external-url">
                      {t("settings.externalUrl")}
                    </Label>
                    <Input
                      id="settings-media-server-external-url"
                      value={draft.externalUrl}
                      onChange={(event) =>
                        setDraft((previous) => ({
                          ...previous,
                          externalUrl: event.target.value,
                        }))
                      }
                      placeholder={`https://${draft.provider}.example.test`}
                    />
                    <p className="mt-1 text-xs text-muted-foreground">
                      {t("settings.externalUrlHint")}
                    </p>
                  </label>
                ) : null}

                <div className="rounded border border-border bg-background/40 p-3">
                  <label className="flex items-center gap-3">
                    <Checkbox
                      id="settings-media-server-enabled"
                      checked={draft.enabled}
                      onCheckedChange={(checked) =>
                        setDraft((previous) => ({
                          ...previous,
                          enabled: checked === true,
                        }))
                      }
                    />
                    <span className="text-sm font-medium">{t("settings.mediaServerEnabled")}</span>
                  </label>
                </div>

                {draft.provider === "PLEX" ? (
                  <div className="space-y-3 rounded border border-border bg-background/40 p-3">
                    <div className="flex flex-wrap items-center gap-3">
                      <Button
                        id="settings-media-server-discover-plex"
                        type="button"
                        variant="outline"
                        onClick={() => void discoverPlexServers()}
                        disabled={plexDiscoveryBusy}
                      >
                        {plexDiscoveryBusy ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <Server className="h-4 w-4" />
                        )}
                        {t("settings.discoverPlexServers")}
                      </Button>
                      {draft.machineIdPresent && !draft.plexServerId ? (
                        <span className="inline-flex items-center gap-1 text-sm text-[var(--scry-success-text)]">
                          <CheckCircle2 className="h-3.5 w-3.5" />
                          {t("settings.plexServerSelected")}
                        </span>
                      ) : null}
                    </div>
                    {plexServerOptions.length > 0 ? (
                      <label className="block">
                        <Label className="mb-2 block" htmlFor="settings-media-server-plex-server">
                          {t("settings.plexServer")}
                        </Label>
                        <Select
                          value={draft.plexServerId}
                          onValueChange={(value) =>
                            setDraft((previous) => ({
                              ...previous,
                              plexServerId: value,
                            }))
                          }
                        >
                          <SelectTrigger id="settings-media-server-plex-server" className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {plexServerOptions.map((server) => (
                              <SelectItem
                                id={selectorId("settings-media-server-plex-server-option", server.id)}
                                key={server.id}
                                value={server.id}
                              >
                                {server.name}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </label>
                    ) : null}
                  </div>
                ) : null}

                {draft.provider === "JELLYFIN" || draft.provider === "EMBY" ? (
                  <div className="space-y-3">
                    {draft.provider === "EMBY" ? (
                      <>
                        <div className="inline-flex rounded-md border border-border p-1">
                          <Button
                            id="settings-emby-mode-local"
                            type="button"
                            variant={draft.embyConnectionMode === "LOCAL" ? "default" : "ghost"}
                            size="sm"
                            onClick={() =>
                              setDraft((previous) => ({
                                ...previous,
                                embyConnectionMode: "LOCAL",
                                embyConnectUsernameOrEmail: "",
                                embyConnectPassword: "",
                                embyConnectServerId: "",
                                embyDiscoveredServers: [],
                              }))
                            }
                          >
                            Local
                          </Button>
                          <Button
                            id="settings-emby-mode-connect"
                            type="button"
                            variant={draft.embyConnectionMode === "CONNECT" ? "default" : "ghost"}
                            size="sm"
                            onClick={() =>
                              setDraft((previous) => ({
                                ...previous,
                                embyConnectionMode: "CONNECT",
                                embyConnectEnabled: true,
                                apiKey: "",
                                adminUsername: "",
                                adminPassword: "",
                              }))
                            }
                          >
                            Connect
                          </Button>
                        </div>

                        {draft.embyConnectionMode === "LOCAL" ? (
                          <div className="inline-flex rounded-md border border-border p-1">
                            <Button
                              id="settings-emby-setup-api-key"
                              type="button"
                              variant={draft.embyLocalSetupMethod === "API_KEY" ? "default" : "ghost"}
                              size="sm"
                              onClick={() =>
                                setDraft((previous) => ({
                                  ...previous,
                                  embyLocalSetupMethod: "API_KEY",
                                  adminUsername: "",
                                  adminPassword: "",
                                }))
                              }
                            >
                              {t("settings.setupViaApiKey")}
                            </Button>
                            <Button
                              id="settings-emby-setup-admin-credentials"
                              type="button"
                              variant={
                                draft.embyLocalSetupMethod === "ADMIN_CREDENTIALS"
                                  ? "default"
                                  : "ghost"
                              }
                              size="sm"
                              onClick={() =>
                                setDraft((previous) => ({
                                  ...previous,
                                  embyLocalSetupMethod: "ADMIN_CREDENTIALS",
                                  apiKey: "",
                                  clearApiKey: false,
                                }))
                              }
                            >
                              {t("settings.loginAsAdmin")}
                            </Button>
                          </div>
                        ) : (
                          <div className="space-y-3 rounded border border-border bg-background/40 p-3">
                            <p className="text-xs text-muted-foreground">
                              The selected Emby Connect user must be a local administrator for setup.
                            </p>
                            <div className="grid gap-3 md:grid-cols-2">
                              <label>
                                <Label className="mb-2 block" htmlFor="settings-emby-connect-username">
                                  Emby Connect username or email
                                </Label>
                                <Input
                                  id="settings-emby-connect-username"
                                  value={draft.embyConnectUsernameOrEmail}
                                  onChange={(event) =>
                                    setDraft((previous) => ({
                                      ...previous,
                                      embyConnectUsernameOrEmail: event.target.value,
                                    }))
                                  }
                                  autoComplete="off"
                                />
                              </label>
                              <label>
                                <Label className="mb-2 block" htmlFor="settings-emby-connect-password">
                                  Password
                                </Label>
                                <Input
                                  id="settings-emby-connect-password"
                                  value={draft.embyConnectPassword}
                                  onChange={(event) =>
                                    setDraft((previous) => ({
                                      ...previous,
                                      embyConnectPassword: event.target.value,
                                    }))
                                  }
                                  type="password"
                                  autoComplete="off"
                                />
                              </label>
                            </div>
                            <Button
                              id="settings-emby-connect-discover"
                              type="button"
                              variant="outline"
                              disabled={embyConnectBusy}
                              onClick={() => void discoverEmbyConnectServers()}
                            >
                              {embyConnectBusy ? "Discovering…" : "Discover servers"}
                            </Button>
                            {draft.embyDiscoveredServers.length > 0 ? (
                              <label>
                                <Label className="mb-2 block" htmlFor="settings-emby-connect-server">
                                  Emby server
                                </Label>
                                <Select
                                  value={draft.embyConnectServerId}
                                  onValueChange={(value) => {
                                    const selected = draft.embyDiscoveredServers.find(
                                      (server) => server.serverId === value,
                                    );
                                    setDraft((previous) => ({
                                      ...previous,
                                      embyConnectServerId: value,
                                      baseUrl: selected?.suggestedBaseUrl ?? previous.baseUrl,
                                    }));
                                  }}
                                >
                                  <SelectTrigger id="settings-emby-connect-server">
                                    <SelectValue placeholder="Select a server" />
                                  </SelectTrigger>
                                  <SelectContent>
                                    {draft.embyDiscoveredServers.map((server) => (
                                      <SelectItem key={server.serverId} value={server.serverId}>
                                        {server.name} · local {server.localStatus.toLowerCase()} · remote{" "}
                                        {server.remoteStatus.toLowerCase()}
                                      </SelectItem>
                                    ))}
                                  </SelectContent>
                                </Select>
                              </label>
                            ) : null}
                          </div>
                        )}

                        <label className="flex items-center gap-3 rounded border border-border bg-background/40 p-3">
                          <Checkbox
                            id="settings-emby-connect-enabled"
                            checked={draft.embyConnectEnabled}
                            onCheckedChange={(checked) =>
                              setDraft((previous) => ({
                                ...previous,
                                embyConnectEnabled: checked === true,
                              }))
                            }
                          />
                          <span>Allow users to sign in through Emby Connect</span>
                        </label>
                        {editingConnectionId && draft.embyConnectEnabled ? (
                          <Button
                            id="settings-emby-test-connect"
                            type="button"
                            variant="outline"
                            disabled={embyConnectBusy}
                            onClick={() => void testSavedEmbyConnect()}
                          >
                            Test Connect
                          </Button>
                        ) : null}
                      </>
                    ) : null}

                    {draft.provider === "JELLYFIN" ? (
                      <div className="inline-flex rounded-md border border-border p-1">
                        <Button
                          id="settings-media-server-credential-admin-login"
                          type="button"
                          variant={draft.jellyfinCredentialMode === "adminLogin" ? "default" : "ghost"}
                          size="sm"
                          onClick={() =>
                            setDraft((previous) => ({
                              ...previous,
                              jellyfinCredentialMode: "adminLogin",
                              apiKey: "",
                              clearApiKey: false,
                            }))
                          }
                        >
                          {t("settings.loginAsAdmin")}
                        </Button>
                        <Button
                          id="settings-media-server-credential-api-key"
                          type="button"
                          variant={draft.jellyfinCredentialMode === "apiKey" ? "default" : "ghost"}
                          size="sm"
                          onClick={() =>
                            setDraft((previous) => ({
                              ...previous,
                              jellyfinCredentialMode: "apiKey",
                              adminUsername: "",
                              adminPassword: "",
                            }))
                          }
                        >
                          {t("settings.setupViaApiKey")}
                        </Button>
                      </div>
                    ) : null}
                    <div className="grid gap-3 md:grid-cols-2">
                      {(draft.provider === "JELLYFIN" && draft.jellyfinCredentialMode === "apiKey") ||
                      (draft.provider === "EMBY" &&
                        draft.embyConnectionMode === "LOCAL" &&
                        draft.embyLocalSetupMethod === "API_KEY") ? (
                        <>
                          <label>
                            <Label
                              className="mb-2 block"
                              htmlFor={
                                draft.provider === "EMBY"
                                  ? "settings-emby-api-key"
                                  : "settings-media-server-api-key"
                              }
                            >
                              {t("settings.apiKey")}
                            </Label>
                            <Input
                              id={
                                draft.provider === "EMBY"
                                  ? "settings-emby-api-key"
                                  : "settings-media-server-api-key"
                              }
                              value={draft.apiKey}
                              onChange={(event) =>
                                setDraft((previous) => ({
                                  ...previous,
                                  apiKey: event.target.value,
                                  clearApiKey: false,
                                }))
                              }
                              type="password"
                              placeholder={t("form.apiKeyInputPlaceholder")}
                            />
                          </label>
                          {editingConnectionId && draft.provider === "JELLYFIN" ? (
                            <label className="flex items-end gap-2 pb-2">
                              <Checkbox
                                checked={draft.clearApiKey}
                                onCheckedChange={(checked) =>
                                  setDraft((previous) => ({
                                    ...previous,
                                    clearApiKey: checked === true,
                                  }))
                                }
                              />
                              <span className="text-sm">{t("settings.clearSavedApiKey")}</span>
                            </label>
                          ) : null}
                        </>
                      ) : (draft.provider === "JELLYFIN" ||
                          (draft.provider === "EMBY" &&
                            draft.embyConnectionMode === "LOCAL" &&
                            draft.embyLocalSetupMethod === "ADMIN_CREDENTIALS")) ? (
                        <>
                          <label>
                            <Label
                              className="mb-2 block"
                              htmlFor={
                                draft.provider === "EMBY"
                                  ? "settings-emby-admin-username"
                                  : "settings-media-server-admin-username"
                              }
                            >
                              {t("settings.adminUsername")}
                            </Label>
                            <Input
                              id={
                                draft.provider === "EMBY"
                                  ? "settings-emby-admin-username"
                                  : "settings-media-server-admin-username"
                              }
                              value={draft.adminUsername}
                              onChange={(event) =>
                                setDraft((previous) => ({
                                  ...previous,
                                  adminUsername: event.target.value,
                                }))
                              }
                              autoComplete="off"
                              placeholder={t("form.usernamePlaceholder")}
                            />
                          </label>
                          <label>
                            <Label
                              className="mb-2 block"
                              htmlFor={
                                draft.provider === "EMBY"
                                  ? "settings-emby-admin-password"
                                  : "settings-media-server-admin-password"
                              }
                            >
                              {t("settings.adminPassword")}
                            </Label>
                            <Input
                              id={
                                draft.provider === "EMBY"
                                  ? "settings-emby-admin-password"
                                  : "settings-media-server-admin-password"
                              }
                              value={draft.adminPassword}
                              onChange={(event) =>
                                setDraft((previous) => ({
                                  ...previous,
                                  adminPassword: event.target.value,
                                }))
                              }
                              type="password"
                              autoComplete="off"
                              placeholder={t("form.passwordPlaceholder")}
                            />
                          </label>
                        </>
                      ) : null}
                    </div>
                    <div>
                      <LocalRemotePathMappingsField
                        fieldKey="path_mappings"
                        label={t("settings.mediaServerPathMappings")}
                        value={draft.pathMappingsText}
                        helpText={t("settings.mediaServerPathMappingsHelp")}
                        direction="remote-to-local"
                        localPathStyle={localPathStyle}
                        onValidityChange={onPathMappingsValidityChange}
                        onChange={(_, value) =>
                          setDraft((previous) => ({
                            ...previous,
                            pathMappingsText: value,
                          }))
                        }
                      />
                    </div>
                  </div>
                ) : null}

                {supportsAuth ? (
                  <div className="space-y-3 rounded border border-border bg-background/40 p-3">
                    <div className="font-medium">{t("settings.mediaServerAuthCapabilities")}</div>
                    <div className="grid gap-3 md:grid-cols-3">
                      <label
                        className={cn(
                          "flex items-center gap-3",
                          !effectiveFormLoginEnabled && "text-muted-foreground",
                        )}
                      >
                        <Checkbox
                          id="settings-media-server-login-enabled"
                          className="size-8 rounded-md"
                          checked={effectiveFormLoginEnabled && draft.loginEnabled}
                          disabled={!effectiveFormLoginEnabled}
                          onCheckedChange={(checked) =>
                            setDraft((previous) => ({
                              ...previous,
                              loginEnabled: effectiveFormLoginEnabled && checked === true,
                            }))
                          }
                        />
                        <span>{t("settings.externalAuthLoginEnabled")}</span>
                      </label>
                      <label className="flex items-center gap-3">
                        <Checkbox
                          id="settings-media-server-linking-enabled"
                          className="size-8 rounded-md"
                          checked={draft.linkingEnabled}
                          onCheckedChange={(checked) =>
                            setDraft((previous) => ({
                              ...previous,
                              linkingEnabled: checked === true,
                            }))
                          }
                        />
                        <span>{t("settings.externalAuthLinkingEnabled")}</span>
                      </label>
                      <label className="flex items-center gap-3">
                        <Checkbox
                          id="settings-media-server-auto-add-enabled"
                          className="size-8 rounded-md"
                          checked={draft.autoAddEnabled}
                          onCheckedChange={(checked) =>
                            setDraft((previous) => ({
                              ...previous,
                              autoAddEnabled: checked === true,
                            }))
                          }
                        />
                        <span>{t("settings.mediaServerAutoAddEnabled")}</span>
                      </label>
                    </div>

                    {!effectiveFormLoginEnabled ? (
                      <p className="text-xs text-muted-foreground">
                        {t("settings.mediaServerLoginRequiresFormLogin")}{" "}
                        <Link
                          to={formLoginSettingsPath}
                          className="font-medium text-primary underline-offset-4 hover:underline"
                        >
                          {t("settings.openSecuritySettings")}
                        </Link>
                      </p>
                    ) : null}

                    {draft.autoAddEnabled ? (
                      <div className="rounded border border-border bg-card/50 p-3">
                        <Label className="mb-2 block">Default Permissions</Label>
                        <PermissionDropdowns
                          libraries={libraries}
                          idPrefix="settings-media-server-default-permissions"
                          selectedAppPermissions={draft.defaultAppPermissions}
                          selectedLibraryPermissions={Object.fromEntries(
                            draft.defaultLibraryGrants.map((grant) => [
                              grant.libraryId,
                              grant.permissions,
                            ]),
                          ) as LibraryPermissionDrafts}
                          onAppChange={(nextPermissions) =>
                            setDraft((previous) => ({
                              ...previous,
                              defaultAppPermissions: nextPermissions,
                            }))
                          }
                          onLibraryChange={(changes) =>
                            setDraft((previous) => ({
                              ...previous,
                              defaultLibraryGrants: Object.entries(changes).reduce(
                                (grants, [libraryId, permissions]) =>
                                  updateLibraryGrant(grants, libraryId, permissions),
                                previous.defaultLibraryGrants,
                              ),
                            }))
                          }
                        />
                      </div>
                    ) : null}
                  </div>
                ) : null}

                {editorError ? (
                  <div
                    role="alert"
                    className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
                  >
                    {editorError}
                  </div>
                ) : null}

                <div className="flex gap-2">
                  <Button
                    id="settings-media-server-save"
                    type="submit"
                    disabled={isSavingEditor || !pathMappingsValid}
                  >
                    {isSavingEditor
                      ? t("label.saving")
                      : isEditing
                        ? t("settings.mediaServerUpdate")
                        : t("settings.mediaServerCreate")}
                  </Button>
                  <Button
                    id="settings-media-server-cancel"
                    type="button"
                    variant="outline"
                    onClick={resetDraft}
                  >
                    {t("label.cancel")}
                  </Button>
                </div>
              </form>
            </CardContent>
          </Card>
          {isEditing ? (
            <div className="flex justify-center">
              <AddNewButton
                id="settings-media-server-create"
                icon={Plus}
                label={t("settings.mediaServerCreateNew")}
                onClick={startCreateConnection}
                disabled={mutatingConnectionId !== null}
              />
            </div>
          ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-media-server-create"
            icon={Plus}
            label={t("settings.mediaServerCreateNew")}
            onClick={startCreateConnection}
          />
        </div>
      )}
    </div>
  );
}

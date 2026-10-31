import * as React from "react";
import {
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Edit,
  Plus,
  Trash2,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { DownloadClientConfigField } from "@/components/common/download-client-config-field";
import { DownloadClientRemotePathMappingsField } from "@/components/common/download-client-remote-path-mappings-field";
import {
  PluginLogo,
  PluginVisualLabel,
} from "@/components/common/plugin-visual";
import { InfoHelp } from "@/components/common/info-help";
import { ProxyAssignmentSelect } from "@/components/common/proxy-assignment-select";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Input,
  integerInputProps,
  sanitizeDigits,
} from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  buildWeaverApiKeyUrl,
  buildUrlPreview,
  FIXED_DOWNLOAD_CLIENT_CONFIG_FIELD_KEYS,
  defaultDownloadClientConfigValuesForFields,
  downloadClientConfigFieldValue,
  isBuiltInDownloadClientConfigField,
  isHostlessDownloadClientType,
} from "@/lib/utils/download-clients";
import { DEFAULT_PORT_FOR_CLIENT_TYPE } from "@/lib/constants/download-clients";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { IconButton } from "@/components/ui/icon-button";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  DownloadClientRecord,
  DownloadClientDraft,
  DownloadClientRoutingSettings,
  DownloadClientRoutingSettingsByScope,
  DownloadClientTypeOption,
  ProxyRecord,
} from "@/lib/types";
import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import { DOWNLOAD_CLIENT_ROUTING_EMPTY } from "@/lib/constants/nzbget";
import { LoadingMark } from "@/components/common/loading-mark";
import { useSeedingProfileOptions } from "@/lib/hooks/use-seeding-profile-options";
import {
  SEEDING_PROFILE_INHERIT_VALUE,
  seedingProfileSelectValue,
  seedingProfileSelectValueToId,
} from "@/lib/utils/seeding-profiles";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";
import type { BoxedActionButtonTone } from "@/lib/utils/action-button-styles";
import type { LocalPathStyle } from "@/lib/utils/local-path-style";

function DownloadClientActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<typeof IconButton>, "tone"> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "edit" | "delete">;
}) {
  return (
    <IconButton label={label} tone={tone} className={className} {...props}>
      {children}
    </IconButton>
  );
}

function formatRelativeTime(isoDate: string): string {
  const date = new Date(isoDate);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const isFuture = diffMs < 0;
  const absMs = Math.abs(diffMs);
  const minutes = Math.max(1, Math.floor(absMs / 60000));
  const hours = Math.floor(absMs / 3600000);
  const days = Math.floor(absMs / 86400000);

  const amount =
    minutes < 60 ? `${minutes}m` : hours < 24 ? `${hours}h` : `${days}d`;

  return isFuture ? `in ${amount}` : `${amount} ago`;
}

function DownloadClientStatusCell({
  client,
}: {
  client: DownloadClientRecord;
}) {
  const t = useTranslate();
  if (!client.isEnabled) {
    return <span className="text-muted-foreground">{t("label.disabled")}</span>;
  }

  if (
    client.lastError ||
    client.status === "error" ||
    client.status === "failed"
  ) {
    return (
      <span
        className="text-[var(--scry-danger-text-soft)]"
        title={client.lastError ?? client.status}
      >
        {t("settings.downloadClientLastError")}
      </span>
    );
  }

  if (client.lastSeenAt) {
    return (
      <span title={client.lastSeenAt}>
        {t("settings.downloadClientLastSeen", {
          time: formatRelativeTime(client.lastSeenAt),
        })}
      </span>
    );
  }

  return (
    <span className="text-muted-foreground">
      {t("settings.downloadClientNoActivity")}
    </span>
  );
}

/// The proxy carrying a client's traffic, or "direct". An assignment whose
/// proxy is gone is called out rather than shown as no proxy at all.
function DownloadClientProxyCell({
  client,
  proxiesById,
}: {
  client: DownloadClientRecord;
  proxiesById: Map<string, ProxyRecord>;
}) {
  const t = useTranslate();
  const assigned = client.proxyConfigId
    ? (proxiesById.get(client.proxyConfigId) ?? null)
    : null;

  if (assigned) {
    return (
      <span
        className={cn(
          "font-medium",
          !assigned.isEnabled && "text-[var(--scry-warning-text)]",
        )}
      >
        {assigned.name}
      </span>
    );
  }

  if (client.proxyConfigId) {
    return (
      <span className="text-[var(--scry-warning-text)]">
        {t("settings.proxyMissing")}
      </span>
    );
  }

  return (
    <span className="text-muted-foreground">{t("settings.proxyDirect")}</span>
  );
}

const DOWNLOAD_CLIENT_ROUTING_FACETS: Array<{
  scope: ViewCategoryId;
  labelKey: "search.facetMovie" | "search.facetSeries" | "search.facetAnime";
}> = [
  { scope: "MOVIE", labelKey: "search.facetMovie" },
  { scope: "SERIES", labelKey: "search.facetSeries" },
  { scope: "ANIME", labelKey: "search.facetAnime" },
];

const DOWNLOAD_CLIENT_PRIORITY_OPTIONS = [
  { value: "force", label: "settings.downloadClientPriorityForce" },
  { value: "very high", label: "settings.downloadClientPriorityVeryHigh" },
  { value: "high", label: "settings.downloadClientPriorityHigh" },
  { value: "normal", label: "settings.downloadClientPriorityNormal" },
  { value: "low", label: "settings.downloadClientPriorityLow" },
  { value: "very low", label: "settings.downloadClientPriorityVeryLow" },
] as const;

function normalizeDownloadClientRoutingPriority(value: string): string {
  const normalized = value.trim().toLowerCase().replaceAll("_", " ");
  return DOWNLOAD_CLIENT_PRIORITY_OPTIONS.some(
    (option) => option.value === normalized,
  )
    ? normalized
    : "normal";
}

function supportsDownloadClientRoutingSeedingProfile(
  clientType: string,
): boolean {
  return ["qbittorrent", "deluge", "transmission", "rtorrent"].includes(
    clientType.trim().toLowerCase(),
  );
}

function DownloadClientRoutingDisclosure({
  downloadClient,
  routingByScope,
  isLoading,
  mutatingScopes,
  onChange,
}: {
  downloadClient: DownloadClientRecord;
  routingByScope: DownloadClientRoutingSettingsByScope;
  isLoading: boolean;
  mutatingScopes: ReadonlySet<ViewCategoryId>;
  onChange: (
    scope: ViewCategoryId,
    clientId: string,
    nextValue: Partial<DownloadClientRoutingSettings>,
    options?: { save?: boolean },
  ) => Promise<void> | void;
}) {
  const t = useTranslate();
  const { options: seedingProfileOptions } = useSeedingProfileOptions();
  const supportsSeedingProfile = supportsDownloadClientRoutingSeedingProfile(
    downloadClient.clientType,
  );

  return (
    <Table
      overflow="auto"
      layout="fixed"
      density="dense"
      wrapperClassName="rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]"
      className="min-w-[1080px] [&_td]:align-middle [&_th]:align-middle"
    >
      <TableHeader>
        <TableRow>
          <TableHead className="w-28">{t("label.name")}</TableHead>
          <TableHead className="w-24 text-center">
            {t("settings.downloadClientRoutingEnabled")}
          </TableHead>
          <TableHead>{t("settings.downloadClientCategory")}</TableHead>
          <TableHead className="w-36">
            {t("settings.downloadClientRecentPriority")}
          </TableHead>
          <TableHead className="w-36">
            {t("settings.downloadClientOlderPriority")}
          </TableHead>
          <TableHead className="w-28 text-center">
            {t("settings.downloadClientRemoveCompleted")}
          </TableHead>
          <TableHead className="w-28 text-center">
            {t("settings.downloadClientRemoveFailed")}
          </TableHead>
          {supportsSeedingProfile ? (
            <TableHead className="w-44">
              {t("settings.seedingProfileColumn")}
            </TableHead>
          ) : null}
        </TableRow>
      </TableHeader>
      <TableBody>
        {DOWNLOAD_CLIENT_ROUTING_FACETS.map(({ scope, labelKey }) => {
          const routing =
            routingByScope[scope]?.[downloadClient.id] ??
            DOWNLOAD_CLIENT_ROUTING_EMPTY;
          const isPending = isLoading || mutatingScopes.has(scope);
          const facetLabel = t(labelKey);

          return (
            <TableRow
              key={scope}
              data-ui="settings-table-row"
              data-subtable-row="download-client-routing"
            >
              <TableCell className="font-medium">{facetLabel}</TableCell>
              <TableCell className="text-center">
                <Checkbox
                  id={selectorId(
                    "settings-download-client-routing-enabled",
                    downloadClient.id,
                    scope,
                  )}
                  size="large"
                  checked={routing.enabled}
                  disabled={isPending}
                  aria-label={`${t("settings.downloadClientRoutingEnabled")}: ${facetLabel}`}
                  onCheckedChange={(checked) =>
                    void onChange(scope, downloadClient.id, {
                      enabled: checked === true,
                    })
                  }
                />
              </TableCell>
              <TableCell>
                <Input
                  id={selectorId(
                    "settings-download-client-routing-category",
                    downloadClient.id,
                    scope,
                  )}
                  value={routing.category}
                  disabled={isPending}
                  placeholder={t("settings.downloadClientCategoryPlaceholder")}
                  onChange={(event) =>
                    void onChange(
                      scope,
                      downloadClient.id,
                      { category: event.target.value },
                      { save: false },
                    )
                  }
                  onBlur={(event) =>
                    void onChange(scope, downloadClient.id, {
                      category: event.target.value,
                    })
                  }
                />
              </TableCell>
              <TableCell>
                <Select
                  value={normalizeDownloadClientRoutingPriority(
                    routing.recentQueuePriority,
                  )}
                  disabled={isPending}
                  onValueChange={(value) =>
                    void onChange(scope, downloadClient.id, {
                      recentQueuePriority: value,
                    })
                  }
                >
                  <SelectTrigger
                    id={selectorId(
                      "settings-download-client-routing-recent-priority",
                      downloadClient.id,
                      scope,
                    )}
                    className="w-full"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {DOWNLOAD_CLIENT_PRIORITY_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {t(option.label)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </TableCell>
              <TableCell>
                <Select
                  value={normalizeDownloadClientRoutingPriority(
                    routing.olderQueuePriority,
                  )}
                  disabled={isPending}
                  onValueChange={(value) =>
                    void onChange(scope, downloadClient.id, {
                      olderQueuePriority: value,
                    })
                  }
                >
                  <SelectTrigger
                    id={selectorId(
                      "settings-download-client-routing-older-priority",
                      downloadClient.id,
                      scope,
                    )}
                    className="w-full"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {DOWNLOAD_CLIENT_PRIORITY_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {t(option.label)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </TableCell>
              <TableCell className="text-center">
                <Checkbox
                  id={selectorId(
                    "settings-download-client-routing-remove-completed",
                    downloadClient.id,
                    scope,
                  )}
                  size="large"
                  checked={routing.removeCompleted}
                  disabled={isPending}
                  aria-label={`${t("settings.downloadClientRemoveCompleted")}: ${facetLabel}`}
                  onCheckedChange={(checked) =>
                    void onChange(scope, downloadClient.id, {
                      removeCompleted: checked === true,
                    })
                  }
                />
              </TableCell>
              <TableCell className="text-center">
                <Checkbox
                  id={selectorId(
                    "settings-download-client-routing-remove-failed",
                    downloadClient.id,
                    scope,
                  )}
                  size="large"
                  checked={routing.removeFailed}
                  disabled={isPending}
                  aria-label={`${t("settings.downloadClientRemoveFailed")}: ${facetLabel}`}
                  onCheckedChange={(checked) =>
                    void onChange(scope, downloadClient.id, {
                      removeFailed: checked === true,
                    })
                  }
                />
              </TableCell>
              {supportsSeedingProfile ? (
                <TableCell>
                  <Select
                    value={seedingProfileSelectValue(routing.seedingProfileId)}
                    disabled={isPending}
                    onValueChange={(value) =>
                      void onChange(scope, downloadClient.id, {
                        seedingProfileId: seedingProfileSelectValueToId(value),
                      })
                    }
                  >
                    <SelectTrigger
                      id={selectorId(
                        "settings-download-client-routing-seeding-profile",
                        downloadClient.id,
                        scope,
                      )}
                      className="w-full"
                      aria-label={t("settings.seedingProfileRoutingLabel", {
                        name: `${downloadClient.name} ${facetLabel}`,
                      })}
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={SEEDING_PROFILE_INHERIT_VALUE}>
                        {t("settings.seedingProfileRoutingInherit")}
                      </SelectItem>
                      {routing.seedingProfileId &&
                      !seedingProfileOptions.some(
                        (option) => option.id === routing.seedingProfileId,
                      ) ? (
                        <SelectItem value={routing.seedingProfileId}>
                          {t("settings.seedingProfileMissing", {
                            id: routing.seedingProfileId,
                          })}
                        </SelectItem>
                      ) : null}
                      {seedingProfileOptions.map((option) => (
                        <SelectItem key={option.id} value={option.id}>
                          {option.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </TableCell>
              ) : null}
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

export type SettingsDownloadClientsSectionProps = {
  editingDownloadClientId: string | null;
  downloadClientTypeOptions: DownloadClientTypeOption[];
  downloadClientDraft: DownloadClientDraft;
  setDownloadClientDraft: React.Dispatch<
    React.SetStateAction<DownloadClientDraft>
  >;
  proxyConfigs: ProxyRecord[];
  submitDownloadClient: (
    event: React.FormEvent<HTMLFormElement>,
  ) => Promise<void> | void;
  testDownloadClientConnection: () => Promise<void>;
  isTestingDownloadClientConnection: boolean;
  mutatingDownloadClientId: string | null;
  resetDownloadClientDraft: () => void;
  settingsDownloadClients: DownloadClientRecord[];
  downloadClientRoutingByScope: DownloadClientRoutingSettingsByScope;
  downloadClientRoutingLoaded: boolean;
  downloadClientRoutingLoading: boolean;
  mutatingDownloadClientRoutingScopes: ReadonlySet<ViewCategoryId>;
  loadDownloadClientRouting: () => Promise<void> | void;
  updateDownloadClientRoutingForScope: (
    scope: ViewCategoryId,
    clientId: string,
    nextValue: Partial<DownloadClientRoutingSettings>,
    options?: { save?: boolean },
  ) => Promise<void> | void;
  editDownloadClient: (downloadClient: DownloadClientRecord) => void;
  toggleDownloadClientEnabled: (
    downloadClient: DownloadClientRecord,
  ) => Promise<void>;
  deleteDownloadClient: (downloadClient: DownloadClientRecord) => Promise<void>;
  downloadClientOrder: string[];
  moveDownloadClient: (
    clientId: string,
    direction: "up" | "down",
  ) => Promise<void> | void;
  isSavingOrder: boolean;
  isEditorOpen: boolean;
  editorMode: "create" | "edit";
  localPathStyle: LocalPathStyle | undefined;
  startCreateDownloadClient: () => void;
};

function DownloadClientTypeOptionContent({
  typeValue,
  label,
  className,
}: {
  typeValue: string;
  label: string;
  className?: string;
}) {
  return (
    <PluginVisualLabel
      providerType={typeValue}
      pluginType="download_client"
      label={label}
      className={className}
    />
  );
}

export function SettingsDownloadClientsSection({
  editingDownloadClientId,
  downloadClientTypeOptions,
  downloadClientDraft,
  setDownloadClientDraft,
  proxyConfigs,
  submitDownloadClient,
  testDownloadClientConnection,
  isTestingDownloadClientConnection,
  mutatingDownloadClientId,
  resetDownloadClientDraft,
  settingsDownloadClients,
  downloadClientRoutingByScope,
  downloadClientRoutingLoaded,
  downloadClientRoutingLoading,
  mutatingDownloadClientRoutingScopes,
  loadDownloadClientRouting,
  updateDownloadClientRoutingForScope,
  editDownloadClient,
  toggleDownloadClientEnabled,
  deleteDownloadClient,
  downloadClientOrder,
  moveDownloadClient,
  isSavingOrder,
  isEditorOpen,
  editorMode,
  localPathStyle,
  startCreateDownloadClient,
}: SettingsDownloadClientsSectionProps) {
  const t = useTranslate();
  const apiKeyInputRef = React.useRef<HTMLInputElement>(null);
  const urlPreview = buildUrlPreview(downloadClientDraft);
  const normalizedClientType = downloadClientDraft.clientType
    .trim()
    .toLowerCase();
  const configuredClientLabel = downloadClientDraft.clientType.trim();
  const selectedDownloadClientTypeOption = downloadClientTypeOptions.find(
    (option) => option.value === normalizedClientType,
  );
  const selectedDownloadClientLabel =
    selectedDownloadClientTypeOption?.label ??
    (configuredClientLabel || "Download client");
  const selectedConfigFields =
    selectedDownloadClientTypeOption?.configFields ?? [];
  const dynamicConfigFields = selectedConfigFields
    .filter((field) => !FIXED_DOWNLOAD_CLIENT_CONFIG_FIELD_KEYS.has(field.key))
    .map((field) =>
      isBuiltInDownloadClientConfigField(normalizedClientType, field)
        ? {
            ...field,
            label: t(field.label),
            helpText: field.helpText ? t(field.helpText) : null,
          }
        : field,
    );
  const isHostless = isHostlessDownloadClientType(normalizedClientType);
  const selectedFieldKeys = new Set(
    selectedConfigFields.map((field) => field.key),
  );
  const hasDescriptorApiKeyField =
    selectedFieldKeys.has("api_key") || selectedFieldKeys.has("apiKey");
  const hasDescriptorCredentialFields =
    selectedFieldKeys.has("username") || selectedFieldKeys.has("password");
  const hasApiKeyField =
    !hasDescriptorApiKeyField &&
    (normalizedClientType === "sabnzbd" || normalizedClientType === "weaver");
  const showCredentialFields =
    !hasDescriptorCredentialFields &&
    (normalizedClientType === "nzbget" ||
      normalizedClientType === "qbittorrent" ||
      normalizedClientType === "sabnzbd");
  const weaverApiKeyUrl =
    normalizedClientType === "weaver"
      ? buildWeaverApiKeyUrl(downloadClientDraft)
      : "";
  const [
    expandedDownloadClientRoutingIds,
    setExpandedDownloadClientRoutingIds,
  ] = React.useState<Set<string>>(() => new Set());
  const toggleDownloadClientRouting = React.useCallback(
    (clientId: string) => {
      const isOpening = !expandedDownloadClientRoutingIds.has(clientId);
      setExpandedDownloadClientRoutingIds((previous) => {
        const next = new Set(previous);
        if (isOpening) {
          next.add(clientId);
        } else {
          next.delete(clientId);
        }
        return next;
      });
      if (isOpening) {
        void loadDownloadClientRouting();
      }
    },
    [expandedDownloadClientRoutingIds, loadDownloadClientRouting],
  );

  const clientById = React.useMemo(
    () => Object.fromEntries(settingsDownloadClients.map((c) => [c.id, c])),
    [settingsDownloadClients],
  );
  const proxiesById = React.useMemo(
    () => new Map(proxyConfigs.map((proxy) => [proxy.id, proxy])),
    [proxyConfigs],
  );
  const editingDownloadClient = editingDownloadClientId
    ? clientById[editingDownloadClientId]
    : null;
  const storedSecretKeys = new Set(
    editingDownloadClient?.storedSecretKeys ?? [],
  );

  const handleDownloadClientTypeChange = React.useCallback(
    (value: string) => {
      setDownloadClientDraft((prev: DownloadClientDraft) => {
        const prevDefault =
          DEFAULT_PORT_FOR_CLIENT_TYPE[prev.clientType] ?? "8080";
        const isDefaultPort = prev.port === "" || prev.port === prevDefault;
        const previousLabel =
          downloadClientTypeOptions.find(
            (option) => option.value === prev.clientType.trim().toLowerCase(),
          )?.label ?? prev.clientType.trim();
        const nextLabel =
          downloadClientTypeOptions.find((option) => option.value === value)
            ?.label ?? value;
        const nextFields =
          downloadClientTypeOptions.find((option) => option.value === value)
            ?.configFields ?? [];
        const shouldAutofillName =
          prev.name.trim().length === 0 || prev.name === previousLabel;
        return {
          ...prev,
          clientType: value,
          name: shouldAutofillName ? nextLabel : prev.name,
          port: isDefaultPort
            ? (DEFAULT_PORT_FOR_CLIENT_TYPE[value] ?? "8080")
            : prev.port,
          configValues: defaultDownloadClientConfigValuesForFields(nextFields),
        };
      });
    },
    [downloadClientTypeOptions, setDownloadClientDraft],
  );
  const orderedClients = React.useMemo(() => {
    if (downloadClientOrder.length === 0) return settingsDownloadClients;
    const ordered: DownloadClientRecord[] = [];
    for (const id of downloadClientOrder) {
      const c = clientById[id];
      if (c) ordered.push(c);
    }
    for (const c of settingsDownloadClients) {
      if (!downloadClientOrder.includes(c.id)) ordered.push(c);
    }
    return ordered;
  }, [downloadClientOrder, clientById, settingsDownloadClients]);
  const hasOptionalCredentials =
    normalizedClientType === "nzbget" || normalizedClientType === "sabnzbd";
  const optionalCredentialLabel = hasOptionalCredentials ? " (optional)" : "";
  const isEditing = editorMode === "edit";
  const [areRemotePathMappingsValid, setAreRemotePathMappingsValid] =
    React.useState(true);
  const [isFilesystemPathMappingOpen, setIsFilesystemPathMappingOpen] =
    React.useState(
      () => downloadClientDraft.remotePathMappings.trim().length > 0,
    );
  const editorIdentity = `${isEditorOpen ? "open" : "closed"}:${editorMode}:${editingDownloadClientId ?? "new"}`;
  const previousEditorIdentity = React.useRef(editorIdentity);

  React.useEffect(() => {
    if (previousEditorIdentity.current === editorIdentity) {
      return;
    }

    previousEditorIdentity.current = editorIdentity;
    setAreRemotePathMappingsValid(true);
    setIsFilesystemPathMappingOpen(
      downloadClientDraft.remotePathMappings.trim().length > 0,
    );
  }, [downloadClientDraft.remotePathMappings, editorIdentity]);

  React.useEffect(() => {
    if (
      downloadClientDraft.remotePathMappings.trim().length > 0 ||
      !areRemotePathMappingsValid
    ) {
      setIsFilesystemPathMappingOpen(true);
    } else {
      setIsFilesystemPathMappingOpen(false);
    }
  }, [areRemotePathMappingsValid, downloadClientDraft.remotePathMappings]);

  const handleSubmitDownloadClient = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      if (!areRemotePathMappingsValid) {
        event.preventDefault();
        setIsFilesystemPathMappingOpen(true);
        return;
      }

      return submitDownloadClient(event);
    },
    [areRemotePathMappingsValid, submitDownloadClient],
  );

  return (
    <div id="settings-download-clients-section" className="space-y-4 text-sm">
      <div
        id="settings-download-clients-table-card"
        className="rounded border border-border"
      >
        <div className="overflow-x-auto">
          <Table id="settings-download-clients-table">
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.downloadClientPriority")}</TableHead>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead className="text-center align-middle">
                  {t("label.type")}
                </TableHead>
                <TableHead>{t("settings.baseUrl")}</TableHead>
                <TableHead>{t("settings.proxyAssignment")}</TableHead>
                <TableHead className="text-center">
                  {t("label.enabled")}
                </TableHead>
                <TableHead>{t("settings.downloadClientStatus")}</TableHead>
                <TableHead className="text-right">
                  {t("label.actions")}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {orderedClients.map((client, index) => {
                const isRoutingExpanded = expandedDownloadClientRoutingIds.has(
                  client.id,
                );
                return (
                  <React.Fragment key={client.id}>
                    <TableRow
                      data-ui="settings-table-row"
                      id={selectorId(
                        "settings-download-client-row",
                        client.name,
                      )}
                      className="cursor-pointer"
                      onClick={(event) => {
                        if (
                          event.target instanceof Element &&
                          event.target.closest(
                            "button, a, input, select, textarea, [role='button']",
                          )
                        ) {
                          return;
                        }
                        toggleDownloadClientRouting(client.id);
                      }}
                    >
                      <TableCell>
                        <div className="flex items-center gap-1">
                          <button
                            type="button"
                            className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                            aria-label={t(
                              "settings.downloadClientRoutingScope",
                              {
                                scope: client.name,
                              },
                            )}
                            aria-controls={selectorId(
                              "settings-download-client-routing",
                              client.id,
                            )}
                            aria-expanded={isRoutingExpanded}
                            onClick={() =>
                              toggleDownloadClientRouting(client.id)
                            }
                          >
                            <ChevronRight
                              className={cn(
                                "h-4 w-4 transition-transform",
                                isRoutingExpanded && "rotate-90",
                              )}
                            />
                          </button>
                          <span className="w-4 text-center text-muted-foreground">
                            {index + 1}
                          </span>
                          <Button
                            id={selectorId(
                              "settings-download-client-move-up",
                              client.name,
                            )}
                            variant="ghost"
                            size="sm"
                            type="button"
                            className="border border-border bg-card/80 hover:bg-accent h-7 w-7 p-0"
                            aria-label={`${t("label.moveUp")} ${client.name}`}
                            onClick={() => moveDownloadClient(client.id, "up")}
                            disabled={isSavingOrder || index === 0}
                          >
                            <ChevronUp className="h-4 w-4" />
                          </Button>
                          <Button
                            id={selectorId(
                              "settings-download-client-move-down",
                              client.name,
                            )}
                            variant="ghost"
                            size="sm"
                            type="button"
                            className="border border-border bg-card/80 hover:bg-accent h-7 w-7 p-0"
                            aria-label={`${t("label.moveDown")} ${client.name}`}
                            onClick={() =>
                              moveDownloadClient(client.id, "down")
                            }
                            disabled={
                              isSavingOrder ||
                              index >= orderedClients.length - 1
                            }
                          >
                            <ChevronDown className="h-4 w-4" />
                          </Button>
                        </div>
                      </TableCell>
                      <TableCell>{client.name}</TableCell>
                      <TableCell className="text-center align-middle">
                        <span className="inline-flex items-center justify-center">
                          <PluginLogo
                            providerType={client.clientType}
                            pluginType="download_client"
                            className="h-5 w-5 rounded-[6px]"
                          />
                          <span className="sr-only">{client.clientType}</span>
                        </span>
                      </TableCell>
                      <TableCell>{client.baseUrl || "—"}</TableCell>
                      <TableCell>
                        <DownloadClientProxyCell
                          client={client}
                          proxiesById={proxiesById}
                        />
                      </TableCell>
                      <TableCell className="text-center">
                        <Checkbox
                          id={selectorId(
                            "settings-download-client-enabled",
                            client.id,
                          )}
                          size="large"
                          checked={client.isEnabled}
                          disabled={mutatingDownloadClientId === client.id}
                          aria-label={`${t("label.enabled")}: ${client.name}`}
                          onCheckedChange={() =>
                            void toggleDownloadClientEnabled(client)
                          }
                        />
                      </TableCell>
                      <TableCell>
                        <DownloadClientStatusCell client={client} />
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="flex justify-end gap-2">
                          <DownloadClientActionButton
                            id={selectorId(
                              "settings-download-client-edit",
                              client.name,
                            )}
                            tone="edit"
                            onClick={() => editDownloadClient(client)}
                            label={t("label.edit")}
                          >
                            <Edit className="h-4 w-4" />
                          </DownloadClientActionButton>
                          <DownloadClientActionButton
                            id={selectorId(
                              "settings-download-client-delete",
                              client.name,
                            )}
                            tone="delete"
                            onClick={() => void deleteDownloadClient(client)}
                            disabled={mutatingDownloadClientId === client.id}
                            label={
                              mutatingDownloadClientId === client.id
                                ? t("label.deleting")
                                : t("label.delete")
                            }
                          >
                            <Trash2 className="h-4 w-4" />
                          </DownloadClientActionButton>
                        </div>
                      </TableCell>
                    </TableRow>
                    {isRoutingExpanded ? (
                      <TableRow
                        id={selectorId(
                          "settings-download-client-routing",
                          client.id,
                        )}
                      >
                        <TableCell colSpan={8} className="bg-muted/20 p-3">
                          {downloadClientRoutingLoaded ? (
                            <div className="mx-auto w-full max-w-6xl">
                              <DownloadClientRoutingDisclosure
                                downloadClient={client}
                                routingByScope={downloadClientRoutingByScope}
                                isLoading={downloadClientRoutingLoading}
                                mutatingScopes={
                                  mutatingDownloadClientRoutingScopes
                                }
                                onChange={updateDownloadClientRoutingForScope}
                              />
                            </div>
                          ) : (
                            <div className="flex min-h-24 items-center justify-center">
                              <LoadingMark className="h-5 w-5" />
                            </div>
                          )}
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </React.Fragment>
                );
              })}
              {orderedClients.length === 0 ? (
                <TableRow id="settings-download-clients-empty-row">
                  <TableCell colSpan={8} className="text-muted-foreground">
                    {t("settings.noDownloadClientsFound")}
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
            <CardHeader className="flex items-center justify-between gap-3">
              <CardTitle
                id="settings-download-client-editor"
                className="text-base"
              >
                {editingDownloadClientId
                  ? t("settings.downloadClientUpdate")
                  : t("settings.downloadClientCreate")}
              </CardTitle>
              <label className="flex shrink-0 items-center gap-3">
                <Checkbox
                  id="settings-download-client-enabled"
                  size="large"
                  checked={downloadClientDraft.isEnabled}
                  disabled={mutatingDownloadClientId !== null}
                  onCheckedChange={(checked) =>
                    setDownloadClientDraft((prev: DownloadClientDraft) => ({
                      ...prev,
                      isEnabled: checked === true,
                    }))
                  }
                />
                <span className="text-sm font-medium">
                  {t("label.enabled")}
                </span>
              </label>
            </CardHeader>
            <CardContent>
              <form
                id="settings-download-client-form"
                className="space-y-3"
                onSubmit={handleSubmitDownloadClient}
              >
                <div className="grid gap-3 md:grid-cols-3">
                  <div>
                    <Label
                      className="mb-2 block"
                      htmlFor="settings-download-client-type"
                    >
                      {t("label.type")}
                    </Label>
                    <Select
                      value={downloadClientDraft.clientType}
                      onValueChange={handleDownloadClientTypeChange}
                    >
                      <SelectTrigger
                        id="settings-download-client-type"
                        className="w-full"
                      >
                        <SelectValue aria-label={selectedDownloadClientLabel}>
                          <DownloadClientTypeOptionContent
                            typeValue={downloadClientDraft.clientType}
                            label={selectedDownloadClientLabel}
                            className="pr-4"
                          />
                        </SelectValue>
                      </SelectTrigger>
                      <SelectContent>
                        {downloadClientTypeOptions.map((option) => (
                          <SelectItem
                            id={selectorId(
                              "settings-download-client-type-option",
                              option.value,
                            )}
                            key={option.value}
                            value={option.value}
                            textValue={option.label}
                          >
                            <DownloadClientTypeOptionContent
                              typeValue={option.value}
                              label={option.label}
                            />
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div>
                    <Label
                      className="mb-2 block"
                      htmlFor="settings-download-client-name"
                    >
                      {t("label.name")}
                    </Label>
                    <Input
                      id="settings-download-client-name"
                      value={downloadClientDraft.name}
                      onChange={(event) =>
                        setDownloadClientDraft((prev: DownloadClientDraft) => ({
                          ...prev,
                          name: event.target.value,
                        }))
                      }
                      required
                      placeholder={t("settings.downloadClientNamePlaceholder")}
                    />
                  </div>
                  {isHostless ? null : (
                    <div className="md:col-span-3 grid grid-cols-1 gap-2 md:grid-cols-[220px_92px_128px_auto] md:items-end md:gap-2">
                      <div>
                        <Label
                          className="mb-2 block"
                          htmlFor="settings-download-client-host"
                        >
                          {t("settings.host")}
                        </Label>
                        <Input
                          id="settings-download-client-host"
                          className="w-56 max-w-56"
                          value={downloadClientDraft.host}
                          onChange={(event) =>
                            setDownloadClientDraft(
                              (prev: DownloadClientDraft) => ({
                                ...prev,
                                host: event.target.value,
                              }),
                            )
                          }
                          required
                          placeholder={t(
                            "settings.downloadClientHostPlaceholder",
                          )}
                        />
                      </div>
                      <div>
                        <Label
                          className="mb-2 block"
                          htmlFor="settings-download-client-port"
                        >
                          {t("settings.port")}
                        </Label>
                        <Input
                          id="settings-download-client-port"
                          {...integerInputProps}
                          value={downloadClientDraft.port}
                          onChange={(event) =>
                            setDownloadClientDraft(
                              (prev: DownloadClientDraft) => ({
                                ...prev,
                                port: sanitizeDigits(event.target.value),
                              }),
                            )
                          }
                          className="w-24 max-w-24"
                          placeholder={t(
                            "settings.downloadClientPortPlaceholder",
                          )}
                        />
                      </div>
                      <div>
                        <Label
                          className="mb-2 block"
                          htmlFor="settings-download-client-url-base"
                        >
                          {t("settings.downloadClientUrlBase")}
                        </Label>
                        <Input
                          id="settings-download-client-url-base"
                          value={downloadClientDraft.urlBase}
                          onChange={(event) =>
                            setDownloadClientDraft(
                              (prev: DownloadClientDraft) => ({
                                ...prev,
                                urlBase: event.target.value,
                              }),
                            )
                          }
                          className="w-36 max-w-36"
                          placeholder={t(
                            "settings.downloadClientUrlBasePlaceholder",
                          )}
                        />
                      </div>
                      <label className="mb-2 ml-2 flex items-center gap-1.5 pl-2.5 md:ml-4">
                        <Checkbox
                          checked={downloadClientDraft.useSsl}
                          onCheckedChange={(checked) =>
                            setDownloadClientDraft(
                              (prev: DownloadClientDraft) => ({
                                ...prev,
                                useSsl: checked === true,
                              }),
                            )
                          }
                        />
                        <span className="inline-flex items-center gap-2 text-sm">
                          {t("settings.downloadClientUseSsl")}
                          <InfoHelp
                            ariaLabel={t("settings.downloadClientUseSsl")}
                            text={t("settings.downloadClientUseSslInfo")}
                          />
                        </span>
                      </label>
                    </div>
                  )}
                  <div className="grid gap-3 md:col-span-3 md:grid-cols-2">
                    {isHostless ? null : (
                      <label>
                        <Label className="mb-2 block">
                          {t("settings.downloadClientUrlPreview")}
                        </Label>
                        <Input
                          value={urlPreview || "https://..."}
                          readOnly
                          disabled
                          className="text-muted-foreground"
                        />
                      </label>
                    )}
                    <div id="settings-download-client-proxy-field">
                      <ProxyAssignmentSelect
                        selectId="settings-download-client-proxy-select"
                        label={t("settings.proxyAssignment")}
                        proxies={proxyConfigs}
                        value={downloadClientDraft.proxyConfigId}
                        onChange={(proxyConfigId) =>
                          setDownloadClientDraft(
                            (prev: DownloadClientDraft) => ({
                              ...prev,
                              proxyConfigId,
                            }),
                          )
                        }
                      />
                    </div>
                  </div>
                  {hasApiKeyField ? (
                    <div>
                      <Label
                        className="mb-2 block"
                        htmlFor="settings-download-client-api-key"
                      >
                        <span className="inline-flex items-center gap-1">
                          {t("settings.apiKey")}
                          {normalizedClientType === "sabnzbd" ? (
                            <InfoHelp
                              ariaLabel={t("settings.apiKey")}
                              text={[
                                t("settings.downloadClientSabnzbdAuthHelp"),
                                t("settings.downloadClientSabnzbdNzbdavHelp"),
                                t(
                                  "settings.downloadClientDecypharrFilesystemHelp",
                                ),
                              ].join(" ")}
                            />
                          ) : null}
                        </span>
                      </Label>
                      <Input
                        ref={apiKeyInputRef}
                        id="settings-download-client-api-key"
                        value={downloadClientDraft.apiKey}
                        onChange={(event) =>
                          setDownloadClientDraft(
                            (prev: DownloadClientDraft) => ({
                              ...prev,
                              apiKey: event.target.value,
                            }),
                          )
                        }
                        placeholder={t("form.apiKeyInputPlaceholder")}
                        type="password"
                        ignorePasswordManagers
                      />
                      {normalizedClientType === "weaver" ? (
                        <p className="mt-2 text-xs text-muted-foreground">
                          Create an integration API key in Weaver:{" "}
                          {weaverApiKeyUrl ? (
                            <a
                              href={weaverApiKeyUrl}
                              target="_blank"
                              rel="noreferrer"
                              onClick={() =>
                                apiKeyInputRef.current?.focus({
                                  preventScroll: true,
                                })
                              }
                              className="underline underline-offset-4 hover:text-foreground"
                            >
                              open Weaver security settings
                            </a>
                          ) : (
                            <span>
                              finish the Weaver URL above to generate the link.
                            </span>
                          )}
                        </p>
                      ) : null}
                    </div>
                  ) : null}
                  {showCredentialFields ? (
                    <>
                      <div>
                        <Label
                          className="mb-2 block"
                          htmlFor="settings-download-client-username"
                        >
                          {t("settings.username")}
                          {optionalCredentialLabel}
                        </Label>
                        <Input
                          id="settings-download-client-username"
                          value={downloadClientDraft.username}
                          onChange={(event) =>
                            setDownloadClientDraft(
                              (prev: DownloadClientDraft) => ({
                                ...prev,
                                username: event.target.value,
                              }),
                            )
                          }
                          placeholder={t("form.usernamePlaceholder")}
                        />
                      </div>
                      <div>
                        <Label
                          className="mb-2 block"
                          htmlFor="settings-download-client-password"
                        >
                          {t("settings.password")}
                          {optionalCredentialLabel}
                        </Label>
                        <Input
                          id="settings-download-client-password"
                          value={downloadClientDraft.password}
                          onChange={(event) =>
                            setDownloadClientDraft(
                              (prev: DownloadClientDraft) => ({
                                ...prev,
                                password: event.target.value,
                              }),
                            )
                          }
                          placeholder={t("form.passwordPlaceholder")}
                          type="password"
                          ignorePasswordManagers
                        />
                      </div>
                      {normalizedClientType === "qbittorrent" ? (
                        <p className="md:col-span-3 text-xs text-muted-foreground">
                          {t("settings.downloadClientQbittorrentDecypharrHelp")}
                        </p>
                      ) : null}
                    </>
                  ) : null}
                  {dynamicConfigFields.map((field) => {
                    const hasStoredSecretValue =
                      storedSecretKeys.has(field.key) &&
                      !Object.hasOwn(
                        downloadClientDraft.configValues,
                        field.key,
                      );
                    return (
                      <div
                        key={field.key}
                        className={
                          field.fieldType === "MULTILINE"
                            ? "md:col-span-3"
                            : undefined
                        }
                      >
                        <DownloadClientConfigField
                          field={field}
                          value={downloadClientConfigFieldValue(
                            downloadClientDraft,
                            field,
                            hasStoredSecretValue,
                          )}
                          hasStoredSecretValue={hasStoredSecretValue}
                          idPrefix="settings-download-client-field"
                          onChange={(key, value) =>
                            setDownloadClientDraft(
                              (prev: DownloadClientDraft) => ({
                                ...prev,
                                configValues: {
                                  ...prev.configValues,
                                  [key]: value,
                                },
                              }),
                            )
                          }
                          onClearStoredSecret={(key) =>
                            setDownloadClientDraft(
                              (prev: DownloadClientDraft) => ({
                                ...prev,
                                configValues: {
                                  ...prev.configValues,
                                  [key]: "",
                                },
                              }),
                            )
                          }
                        />
                      </div>
                    );
                  })}
                  {normalizedClientType === "qbittorrent" ? (
                    <p className="md:col-span-3 text-xs text-muted-foreground">
                      {t("settings.downloadClientDecypharrFilesystemHelp")}
                    </p>
                  ) : null}
                  <details
                    id="settings-download-client-filesystem-path-mapping"
                    className="md:col-span-3 rounded-xl border border-border bg-card p-3"
                    open={isFilesystemPathMappingOpen}
                    onToggle={(event) =>
                      setIsFilesystemPathMappingOpen(event.currentTarget.open)
                    }
                  >
                    <summary
                      id="settings-download-client-filesystem-path-mapping-toggle"
                      className="cursor-pointer select-none text-sm font-medium text-card-foreground"
                    >
                      {t("settings.downloadClientFilesystemPathMapping")}
                    </summary>
                    <div className="mt-3 space-y-3">
                      <p className="text-xs text-muted-foreground">
                        {t("settings.downloadClientFilesystemPathMappingHelp")}
                      </p>
                      <DownloadClientRemotePathMappingsField
                        fieldKey="remote_path_mappings"
                        label={t("settings.downloadClientRemotePathMappings")}
                        value={downloadClientDraft.remotePathMappings}
                        helpText={t(
                          "settings.downloadClientRemotePathMappingsHelp",
                        )}
                        localPathStyle={localPathStyle}
                        onValidityChange={setAreRemotePathMappingsValid}
                        onChange={(_, value) =>
                          setDownloadClientDraft(
                            (prev: DownloadClientDraft) => ({
                              ...prev,
                              remotePathMappings: value,
                            }),
                          )
                        }
                      />
                    </div>
                  </details>
                </div>
                <div className="flex gap-2">
                  <Button
                    id="settings-download-client-save"
                    type="submit"
                    disabled={
                      mutatingDownloadClientId === "new" ||
                      !areRemotePathMappingsValid
                    }
                  >
                    {mutatingDownloadClientId === "new"
                      ? t("label.saving")
                      : editingDownloadClientId
                        ? t("settings.downloadClientUpdate")
                        : t("settings.downloadClientCreate")}
                  </Button>
                  <Button
                    id="settings-download-client-test-connection"
                    type="button"
                    variant="secondary"
                    onClick={() => void testDownloadClientConnection()}
                    disabled={
                      isTestingDownloadClientConnection ||
                      mutatingDownloadClientId !== null ||
                      !areRemotePathMappingsValid
                    }
                  >
                    {isTestingDownloadClientConnection
                      ? t("status.testingDownloadClient", {
                          client: selectedDownloadClientLabel,
                        })
                      : t("label.testConnection")}
                  </Button>
                  <Button
                    id="settings-download-client-cancel"
                    type="button"
                    variant="outline"
                    onClick={resetDownloadClientDraft}
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
                id="settings-download-client-create"
                icon={Plus}
                label={t("settings.downloadClientCreateNew")}
                onClick={startCreateDownloadClient}
                disabled={mutatingDownloadClientId !== null}
              />
            </div>
          ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-download-client-create"
            icon={Plus}
            label={t("settings.downloadClientCreateNew")}
            onClick={startCreateDownloadClient}
          />
        </div>
      )}
    </div>
  );
}

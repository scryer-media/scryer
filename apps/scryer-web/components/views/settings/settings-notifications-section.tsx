
import * as React from "react";
import { Bell, Edit, Loader2, Plus, Power, PowerOff, Send, Trash2 } from "lucide-react";
import { Link } from "react-router";
import { AddNewButton } from "@/components/common/add-new-button";
import { InfoHelp } from "@/components/common/info-help";
import { PluginVisualLabel } from "@/components/common/plugin-visual";
import { TitleAutocompletePicker } from "@/components/common/title-autocomplete-picker";
import { LocalRemotePathMappingsField } from "@/components/common/local-remote-path-mappings-field";
import type { LocalPathStyle } from "@/lib/utils/local-path-style";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox, CheckboxField } from "@/components/ui/checkbox";
import { Input, signedIntegerInputProps } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { MultiSelectDropdown } from "@/components/ui/multi-select-dropdown";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { Translate } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { subscribableProviderNotificationEvents } from "@/lib/notification-capabilities";
import { selectorId } from "@/lib/utils/dom-ids";
import { sectionLabelForFacet } from "@/lib/facets/helpers";
import { viewFromFacet } from "@/lib/facets/helpers";
import type {
  ConfigFieldDef,
  NotificationChannel,
  NotificationChannelDraft,
  NotificationProviderType,
  NotificationSubscriptionDraft,
  NotificationSubscriptionRow,
  NotificationTarget,
  TitleRecord,
} from "@/lib/types";
import type { Facet } from "@/lib/types/titles";
import type { BoxedActionButtonTone } from "@/lib/utils/action-button-styles";
import { buildOverviewDetailPath } from "@/lib/utils/routing";

type SettingsNotificationsSectionProps = {
  channels: NotificationChannel[];
  editingChannelId: string | null;
  channelDraft: NotificationChannelDraft;
  setChannelDraft: React.Dispatch<React.SetStateAction<NotificationChannelDraft>>;
  submitChannel: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mutatingChannelId: string | null;
  resetChannelDraft: () => void;
  editChannel: (channel: NotificationChannel) => void;
  toggleChannelEnabled: (channel: NotificationChannel) => Promise<void> | void;
  deleteChannel: (channel: NotificationChannel) => Promise<void> | void;
  testChannel: (channel: NotificationChannel) => Promise<void> | void;
  testingChannelId: string | null;
  isChannelEditorOpen: boolean;
  channelEditorMode: "create" | "edit";
  startCreateChannel: () => void;
  providerTypes: NotificationProviderType[];
  notificationTargets: NotificationTarget[];
  subscriptions: NotificationSubscriptionRow[];
  subscriptionTitlesById: Record<string, TitleRecord | null>;
  editingSubscriptionId: string | null;
  subscriptionDraft: NotificationSubscriptionDraft;
  setSubscriptionDraft: React.Dispatch<React.SetStateAction<NotificationSubscriptionDraft>>;
  submitSubscription: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mutatingSubscriptionId: string | null;
  resetSubscriptionDraft: () => void;
  editSubscription: (sub: NotificationSubscriptionRow) => void;
  toggleSubscriptionEnabled: (sub: NotificationSubscriptionRow) => Promise<void> | void;
  deleteSubscription: (sub: NotificationSubscriptionRow) => Promise<void> | void;
  eventTypes: string[];
  isSubscriptionEditorOpen: boolean;
  subscriptionEditorMode: "create" | "edit";
  startCreateSubscription: (target?: Pick<NotificationTarget, "targetKind" | "id">) => void;
  localPathStyle: LocalPathStyle | undefined;
};

const SCOPE_OPTIONS = ["global", "facet", "title"] as const;
const FACET_SCOPE_OPTIONS: Facet[] = ["MOVIE", "SERIES", "ANIME"];
const MEDIA_SERVER_PROVIDER_TYPES = new Set(["jellyfin", "plex", "emby"]);

function isMediaServerProviderType(providerType: string): boolean {
  return MEDIA_SERVER_PROVIDER_TYPES.has(providerType.trim().toLowerCase());
}

function NotificationActionButton({
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
    <IconButton
      label={label}
      tone={tone}
      className={className}
      {...props}
    >
      {children}
    </IconButton>
  );
}

const NOTIFICATION_EVENT_LABEL_KEYS: Record<string, string> = {
  grab: "settings.notificationEvent.grab",
  download: "settings.notificationEvent.download",
  upgrade: "settings.notificationEvent.upgrade",
  import_complete: "settings.notificationEvent.importComplete",
  import_rejected: "settings.notificationEvent.importRejected",
  rename: "settings.notificationEvent.rename",
  title_added: "settings.notificationEvent.titleAdded",
  title_deleted: "settings.notificationEvent.titleDeleted",
  file_deleted: "settings.notificationEvent.fileDeleted",
  file_deleted_for_upgrade: "settings.notificationEvent.fileDeletedForUpgrade",
  post_processing_completed: "settings.notificationEvent.postProcessingCompleted",
  subtitle_downloaded: "settings.notificationEvent.subtitleDownloaded",
  subtitle_search_failed: "settings.notificationEvent.subtitleSearchFailed",
  media_request_submitted: "settings.notificationEvent.mediaRequestSubmitted",
  media_request_approved: "settings.notificationEvent.mediaRequestApproved",
  media_request_rejected: "settings.notificationEvent.mediaRequestRejected",
  media_request_canceled: "settings.notificationEvent.mediaRequestCanceled",
  health_issue: "settings.notificationEvent.healthIssue",
  health_restored: "settings.notificationEvent.healthRestored",
  application_update: "settings.notificationEvent.applicationUpdate",
  manual_interaction_required: "settings.notificationEvent.manualInteractionRequired",
  test: "settings.notificationEvent.test",
};

function humanizeSnakeCase(value: string) {
  return value
    .split("_")
    .filter(Boolean)
    .map((segment) => segment.charAt(0).toUpperCase() + segment.slice(1))
    .join(" ");
}

function notificationEventLabel(eventType: string, t: Translate) {
  const key = NOTIFICATION_EVENT_LABEL_KEYS[eventType];
  return key ? t(key) : humanizeSnakeCase(eventType);
}

function notificationScopeLabel(scope: string, t: Translate) {
  const key = `settings.notificationScope.${scope}`;
  const translated = t(key);
  return translated === key ? humanizeSnakeCase(scope) : translated;
}

function isFacetScopeValue(value: string): value is Facet {
  return FACET_SCOPE_OPTIONS.includes(value as Facet);
}

function parseFacetScopeIds(scopeId: string | null | undefined): Facet[] {
  if (!scopeId) return [];

  const selected = new Set<Facet>();
  for (const token of scopeId.split(",")) {
    const normalized = token.trim().toLowerCase();
    if (isFacetScopeValue(normalized)) {
      selected.add(normalized);
    }
  }

  return FACET_SCOPE_OPTIONS.filter((facet) => selected.has(facet));
}

function formatResolvedTitleLabel(title: TitleRecord): string {
  return title.year ? `${title.name} (${title.year})` : title.name;
}

function titleOverviewHref(title: TitleRecord): string {
  const titleSlug = title.slug?.trim() || null;
  const librarySlug = title.librarySlug?.trim() || null;
  const basePath = buildOverviewDetailPath(viewFromFacet(title.facet), librarySlug, titleSlug);
  if (titleSlug && librarySlug) {
    return basePath;
  }
  return `${basePath}?id=${encodeURIComponent(title.id)}`;
}

function notificationScopeIdLabel(
  scope: string,
  scopeId: string | null | undefined,
  t: Translate,
  subscriptionTitlesById: Record<string, TitleRecord | null>,
): string | null {
  if (!scopeId) {
    return null;
  }

  if (scope === "title") {
    const title = subscriptionTitlesById[scopeId];
    return title ? formatResolvedTitleLabel(title) : t("label.unknown");
  }

  if (scope !== "facet") {
    return scopeId;
  }

  const facets = parseFacetScopeIds(scopeId);
  if (facets.length === 0) {
    return scopeId;
  }

  return facets.map((facet) => sectionLabelForFacet(t, facet)).join(", ");
}

function DynamicConfigField({
  field,
  value,
  onChange,
  localPathStyle,
}: {
  field: ConfigFieldDef;
  value: string;
  onChange: (key: string, value: string) => void;
  localPathStyle: LocalPathStyle | undefined;
}) {
  const fieldId = selectorId("settings-notification-field", field.key);
  const help = field.helpText ? (
    <InfoHelp
      text={field.helpText}
      ariaLabel={`About ${field.label}`}
    />
  ) : null;
  const requiredMarker = field.required ? (
    <span aria-hidden="true" className="text-destructive">
      *
    </span>
  ) : null;

  if (field.fieldType === "BOOL") {
    return (
      <CheckboxField
        id={fieldId}
        checked={value === "true"}
        onCheckedChange={(checkedValue) =>
          onChange(field.key, checkedValue === true ? "true" : "false")
        }
        label={field.label}
        labelAccessory={
          <>
            {requiredMarker}
            {help}
          </>
        }
        className="items-center"
        checkboxClassName="mt-0"
      />
    );
  }

  if (field.fieldType === "SELECT" && field.options.length > 0) {
    return (
      <label>
        <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
          {help}
        </Label>
        <Select
          value={value || field.defaultValue || ""}
          onValueChange={(v) => onChange(field.key, v)}
        >
          <SelectTrigger id={fieldId} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {field.options.map((opt) => (
              <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
            ))}
          </SelectContent>
        </Select>
      </label>
    );
  }

  if (field.fieldType === "MULTILINE") {
    if (field.key === "path_mappings") {
      return (
        <LocalRemotePathMappingsField
          fieldKey={field.key}
          label={field.label}
          value={value}
          helpText={field.helpText}
          required={field.required}
          localPathStyle={localPathStyle}
          onChange={onChange}
        />
      );
    }

    return (
      <label>
        <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
          {help}
        </Label>
        <Textarea
          id={fieldId}
          value={value}
          onChange={(e) => onChange(field.key, e.target.value)}
          required={field.required}
          placeholder={field.defaultValue ?? ""}
          rows={6}
        />
      </label>
    );
  }

  return (
    <label>
      <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
        {field.label}
        {requiredMarker}
        {help}
      </Label>
      <Input
        id={fieldId}
        value={value}
        onChange={(e) => onChange(field.key, e.target.value)}
        {...(field.fieldType === "NUMBER" ? signedIntegerInputProps : {})}
        type={
          field.fieldType === "PASSWORD"
            ? "password"
            : field.fieldType === "NUMBER"
              ? "number"
              : "text"
        }
        ignorePasswordManagers={field.fieldType === "PASSWORD"}
        required={field.required}
        placeholder={field.defaultValue ?? ""}
      />
    </label>
  );
}

function notificationTargetValue(target: NotificationTarget): string {
  return `${target.targetKind}:${target.id}`;
}

function parseNotificationTargetValue(value: string): Pick<NotificationTarget, "targetKind" | "id"> | null {
  const separator = value.indexOf(":");
  if (separator <= 0) {
    return null;
  }
  const targetKind = value.slice(0, separator);
  if (targetKind !== "plugin_channel" && targetKind !== "media_server_connection") {
    return null;
  }
  const id = value.slice(separator + 1);
  return id ? { targetKind, id } : null;
}

function notificationTargetName(
  targets: NotificationTarget[],
  targetKind: NotificationSubscriptionRow["targetKind"],
  targetId: string,
): string {
  return (
    targets.find((target) => target.targetKind === targetKind && target.id === targetId)?.name ??
    targetId
  );
}

export function SettingsNotificationsSection({
  channels,
  editingChannelId,
  channelDraft,
  setChannelDraft,
  submitChannel,
  mutatingChannelId,
  resetChannelDraft,
  editChannel,
  toggleChannelEnabled,
  deleteChannel,
  testChannel,
  testingChannelId,
  isChannelEditorOpen,
  channelEditorMode,
  startCreateChannel,
  providerTypes,
  notificationTargets,
  subscriptions,
  subscriptionTitlesById,
  editingSubscriptionId,
  subscriptionDraft,
  setSubscriptionDraft,
  submitSubscription,
  mutatingSubscriptionId,
  resetSubscriptionDraft,
  editSubscription,
  toggleSubscriptionEnabled,
  deleteSubscription,
  eventTypes,
  isSubscriptionEditorOpen,
  subscriptionEditorMode,
  startCreateSubscription,
  localPathStyle,
}: SettingsNotificationsSectionProps) {
  const t = useTranslate();
  const normalizedChannelType = channelDraft.channelType.trim().toLowerCase();
  const isEditingChannel = channelEditorMode === "edit";
  const isEditingSubscription = subscriptionEditorMode === "edit";
  const selectedFacetScopeIds = subscriptionDraft.facetScopeIds;
  const providerByType = React.useMemo(
    () =>
      new Map(
        providerTypes.map((providerType) => [
          providerType.providerType.trim().toLowerCase(),
          providerType,
        ]),
      ),
    [providerTypes],
  );
  const scopeOptions = React.useMemo(
    () =>
      SCOPE_OPTIONS.map((scope) => ({
        value: scope,
        label: notificationScopeLabel(scope, t),
      })),
    [t],
  );
  const selectedSubscriptionTarget = React.useMemo(() => {
    return (
      notificationTargets.find(
        (target) =>
          target.targetKind === subscriptionDraft.targetKind &&
          target.id === subscriptionDraft.targetId,
      ) ?? null
    );
  }, [notificationTargets, subscriptionDraft.targetId, subscriptionDraft.targetKind]);
  const selectedSubscriptionProvider = React.useMemo(() => {
    const providerType = selectedSubscriptionTarget?.providerType.trim().toLowerCase();
    return providerType ? providerByType.get(providerType) ?? null : null;
  }, [providerByType, selectedSubscriptionTarget]);
  const supportedSubscriptionEventTypes = React.useMemo(() => {
    const supportedEvents = selectedSubscriptionProvider?.supportedEvents ?? [];
    return subscribableProviderNotificationEvents(eventTypes, supportedEvents);
  }, [eventTypes, selectedSubscriptionProvider]);
  const eventTypeOptions = React.useMemo(
    () =>
      supportedSubscriptionEventTypes.map((eventType) => ({
        value: eventType,
        label: notificationEventLabel(eventType, t),
      })),
    [supportedSubscriptionEventTypes, t],
  );
  const orderedSubscriptionEventTypes = React.useCallback(
    (values: string[]) => {
      const order = new Map(eventTypes.map((value, index) => [value, index]));
      return [...values].sort((left, right) => {
        const leftIndex = order.get(left) ?? Number.MAX_SAFE_INTEGER;
        const rightIndex = order.get(right) ?? Number.MAX_SAFE_INTEGER;
        if (leftIndex !== rightIndex) {
          return leftIndex - rightIndex;
        }
        return left.localeCompare(right);
      });
    },
    [eventTypes],
  );
  const isSubscriptionDraftValid =
    subscriptionDraft.targetId.trim().length > 0 &&
    subscriptionDraft.eventTypes.length > 0 &&
    subscriptionDraft.scope.trim().length > 0 &&
    (subscriptionDraft.scope !== "facet" || selectedFacetScopeIds.length > 0) &&
    (subscriptionDraft.scope !== "title" ||
      subscriptionDraft.titleScopeId.trim().length > 0);

  const providerTypeOptions = React.useMemo(() => {
    if (providerTypes.length === 0) return [];
    return providerTypes
      .filter((pt) => !isMediaServerProviderType(pt.providerType))
      .map((pt) => ({ value: pt.providerType, label: pt.name }));
  }, [providerTypes]);

  const selectedProvider = React.useMemo(() => {
    return providerByType.get(normalizedChannelType) ?? null;
  }, [normalizedChannelType, providerByType]);

  const selectedProviderFields = selectedProvider?.configFields ?? [];

  React.useEffect(() => {
    const supported = new Set(supportedSubscriptionEventTypes);
    setSubscriptionDraft((prev) => {
      const filtered = orderedSubscriptionEventTypes(
        prev.eventTypes.filter((eventType) => supported.has(eventType)),
      );
      if (
        filtered.length === prev.eventTypes.length
        && filtered.every((eventType, index) => eventType === prev.eventTypes[index])
      ) {
        return prev;
      }
      return {
        ...prev,
        eventTypes: filtered,
      };
    });
  }, [
    orderedSubscriptionEventTypes,
    setSubscriptionDraft,
    supportedSubscriptionEventTypes,
  ]);

  const handleConfigValueChange = React.useCallback(
    (key: string, value: string) => {
      setChannelDraft((prev) => ({
        ...prev,
        configValues: { ...prev.configValues, [key]: value },
      }));
    },
    [setChannelDraft],
  );

  const handleFacetScopeCheckedChange = React.useCallback(
    (facet: Facet, checked: boolean) => {
      setSubscriptionDraft((prev) => {
        const next = new Set(prev.facetScopeIds);
        if (checked) {
          next.add(facet);
        } else {
          next.delete(facet);
        }
        return {
          ...prev,
          facetScopeIds: FACET_SCOPE_OPTIONS.filter((scopeOption) => next.has(scopeOption)),
        };
      });
    },
    [setSubscriptionDraft],
  );

  return (
    <div id="settings-notifications-section" className="space-y-6 text-sm">
      {/* ── Channels ──────────────────────────────────── */}
      <CardTitle className="flex items-center gap-2 text-base">
        <Bell className="h-4 w-4" />
        {t("settings.notificationChannels")}
      </CardTitle>

      <div className="rounded border border-border">
        <div className="overflow-x-auto">
          <Table id="settings-notification-channels-table">
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead>{t("settings.notificationProviderType")}</TableHead>
                <TableHead className="text-center">{t("label.enabled")}</TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {notificationTargets.map((target) => {
                const channel =
                  target.targetKind === "plugin_channel"
                    ? channels.find((item) => item.id === target.id) ?? null
                    : null;
                const provider = providerByType.get(
                  target.providerType.trim().toLowerCase(),
                );
                const providerLabel =
                  provider?.name ??
                  humanizeSnakeCase(target.providerType);
                const targetLabel =
                  target.targetKind === "media_server_connection"
                    ? t("settings.mediaServersSection")
                    : t("settings.notificationChannel");
                return (
                  <TableRow
                    data-ui="settings-table-row"
                    key={notificationTargetValue(target)}
                    id={selectorId("settings-notification-channel-row", target.name)}
                  >
                    <TableCell>
                      <div className="flex flex-wrap items-center gap-2">
                        <span>{target.name}</span>
                        <span className="rounded border border-border bg-muted/40 px-2 py-0.5 text-xs text-muted-foreground">
                          {targetLabel}
                        </span>
                        {channel?.blockedReason ? (
                          <Badge
                            tone="negative"
                            data-ui="settings-notification-channel-blocked"
                            title={channel.blockedReason}
                          >
                            {t("settings.pluginBlocked")}
                          </Badge>
                        ) : null}
                      </div>
                      {channel?.blockedReason ? (
                        <div
                          data-ui="settings-notification-channel-blocked-reason"
                          className="mt-1 whitespace-normal break-words text-xs text-destructive"
                        >
                          {channel.blockedReason}
                        </div>
                      ) : null}
                    </TableCell>
                    <TableCell>
                      <PluginVisualLabel
                        providerType={target.providerType}
                        pluginType={
                          target.targetKind === "media_server_connection"
                            ? "media_server"
                            : "notification"
                        }
                        label={providerLabel}
                      />
                    </TableCell>
                    <TableCell className="text-center">
                      {channel?.blockedReason ? (
                        // Still switched on, still configured, not running: a
                        // checkmark here would claim the opposite.
                        <span
                          data-ui="settings-notification-channel-enabled-blocked"
                          title={channel.blockedReason}
                          aria-label={`${t("settings.pluginBlocked")}: ${target.name}`}
                          className="text-xs font-medium text-destructive"
                        >
                          {t("settings.pluginBlockedNotRunning")}
                        </span>
                      ) : (
                        <RenderBooleanIcon
                          value={target.isEnabled}
                          label={`${t("label.enabled")}: ${target.name}`}
                        />
                      )}
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="inline-flex items-center gap-2">
                        <NotificationActionButton
                          id={selectorId("settings-notification-subscription-create-for", target.name)}
                          label={t("settings.notificationSubscriptionCreate")}
                          tone="neutral"
                          onClick={() => startCreateSubscription(target)}
                          disabled={!target.isEnabled || !provider}
                        >
                          <Plus className="h-4 w-4" />
                        </NotificationActionButton>
                        {channel ? (
                          <>
                            {provider?.supportsTest ? (
                              <NotificationActionButton
                                id={selectorId("settings-notification-channel-test", channel.name)}
                                label={t("settings.notificationTest")}
                                tone="neutral"
                                onClick={() => void testChannel(channel)}
                                disabled={testingChannelId === channel.id}
                              >
                                {testingChannelId === channel.id ? (
                                  <Loader2 className="h-4 w-4 animate-spin" />
                                ) : (
                                  <Send className="h-4 w-4" />
                                )}
                              </NotificationActionButton>
                            ) : null}
                            <NotificationActionButton
                              id={selectorId("settings-notification-channel-toggle", channel.name)}
                              label={channel.isEnabled ? t("label.disable") : t("label.enable")}
                              tone={channel.isEnabled ? "enabled" : "disabled"}
                              onClick={() => void toggleChannelEnabled(channel)}
                              disabled={mutatingChannelId === channel.id}
                            >
                              {channel.isEnabled ? (
                                <Power className="h-4 w-4" />
                              ) : (
                                <PowerOff className="h-4 w-4" />
                              )}
                            </NotificationActionButton>
                            <NotificationActionButton
                              id={selectorId("settings-notification-channel-edit", channel.name)}
                              label={t("label.edit")}
                              tone="edit"
                              onClick={() => editChannel(channel)}
                            >
                              <Edit className="h-4 w-4" />
                            </NotificationActionButton>
                            <NotificationActionButton
                              id={selectorId("settings-notification-channel-delete", channel.name)}
                              label={t("label.delete")}
                              tone="delete"
                              onClick={() => void deleteChannel(channel)}
                              disabled={mutatingChannelId === channel.id}
                            >
                              {mutatingChannelId === channel.id ? (
                                <Loader2 className="h-4 w-4 animate-spin" />
                              ) : (
                                <Trash2 className="h-4 w-4" />
                              )}
                            </NotificationActionButton>
                          </>
                        ) : null}
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
              {notificationTargets.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={4} className="text-muted-foreground">
                    {t("settings.notificationNoChannels")}
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </div>

      {isChannelEditorOpen ? (
        <>
          <Card>
            <CardHeader>
              <CardTitle className="text-base">
                {isEditingChannel
                  ? t("settings.notificationChannelUpdate")
                  : t("settings.notificationChannelCreate")}
              </CardTitle>
            </CardHeader>
            <CardContent>
              {providerTypeOptions.length === 0 ? (
                <div className="space-y-2 text-sm text-muted-foreground">
                  <p>{t("settings.notificationNoProviders")}</p>
                  <p>
                    To notify a media server (ie. Jellyfin), first create a{" "}
                    <Link className="underline hover:text-foreground" to="/settings/media-servers">
                      media server connection
                    </Link>
                    .
                  </p>
                </div>
              ) : (
                <form id="settings-notification-channel-form" className="space-y-3" onSubmit={submitChannel}>
               <div className="grid gap-3 md:grid-cols-2">
                 <label>
                   <Label className="mb-2 block" htmlFor="settings-notification-channel-provider-type">{t("settings.notificationProviderType")}</Label>
                   <Select
                     value={normalizedChannelType || undefined}
                     onValueChange={(v) => {
                       const provider = providerTypes.find((pt) => pt.providerType === v);
                       setChannelDraft((prev) => ({
                         ...prev,
                         channelType: v,
                         mediaServerConnectionId: "",
                         name: provider?.name ?? "",
                       }));
                     }}
                   >
                     <SelectTrigger id="settings-notification-channel-provider-type" className="w-full">
                       <SelectValue placeholder={t("settings.notificationProviderType")}>
                         {selectedProvider ? (
                           <PluginVisualLabel
                             providerType={selectedProvider.providerType}
                             pluginType="notification"
                             label={selectedProvider.name}
                           />
                         ) : null}
                       </SelectValue>
                     </SelectTrigger>
                     <SelectContent>
                      {providerTypeOptions.map((opt) => (
                         <SelectItem
                           id={selectorId("settings-notification-channel-provider-type-option", opt.value)}
                           key={opt.value}
                           value={opt.value}
                         >
                           <PluginVisualLabel
                             providerType={opt.value}
                             pluginType="notification"
                             label={opt.label}
                           />
                         </SelectItem>
                       ))}
                     </SelectContent>
                   </Select>
                 </label>
                 <label>
                   <Label className="mb-2 block" htmlFor="settings-notification-channel-name">{t("label.name")}</Label>
                   <Input
                     id="settings-notification-channel-name"
                     value={channelDraft.name}
                     onChange={(event) =>
                       setChannelDraft((prev) => ({
                         ...prev,
                         name: event.target.value,
                       }))
                     }
                     required
                     placeholder="My Webhook"
                   />
                 </label>
               </div>

              {selectedProviderFields.length > 0 ? (
                <div className="space-y-3">
                  <div className="grid gap-3 md:grid-cols-2">
                    {selectedProviderFields
                      .filter((f) => f.fieldType !== "BOOL")
                      .map((field) => (
                        <DynamicConfigField
                          key={field.key}
                          field={field}
                          value={channelDraft.configValues[field.key] ?? field.defaultValue ?? ""}
                          onChange={handleConfigValueChange}
                          localPathStyle={localPathStyle}
                        />
                      ))}
                  </div>
                  {selectedProviderFields.some((f) => f.fieldType === "BOOL") ? (
                    <div className="flex items-center gap-6">
                      {selectedProviderFields
                        .filter((f) => f.fieldType === "BOOL")
                        .map((field) => (
                          <DynamicConfigField
                            key={field.key}
                            field={field}
                            value={channelDraft.configValues[field.key] ?? field.defaultValue ?? "false"}
                            onChange={handleConfigValueChange}
                            localPathStyle={localPathStyle}
                          />
                        ))}
                    </div>
                  ) : null}
                </div>
              ) : null}

              <label className="flex items-center gap-2">
                <Checkbox
                  id="settings-notification-channel-enabled"
                  checked={channelDraft.isEnabled}
                  onCheckedChange={(value) =>
                    setChannelDraft((prev) => ({
                      ...prev,
                      isEnabled: value === true,
                    }))
                  }
                />
                <span className="text-sm">{t("label.enabled")}</span>
              </label>

              <div className="flex gap-2">
                <Button id="settings-notification-channel-save" type="submit" disabled={mutatingChannelId === "new"}>
                  {mutatingChannelId === "new"
                    ? t("label.saving")
                    : editingChannelId
                      ? t("settings.notificationChannelUpdate")
                      : t("settings.notificationChannelCreate")}
                </Button>
                <Button id="settings-notification-channel-cancel" type="button" variant="outline" onClick={resetChannelDraft}>
                  {t("label.cancel")}
                </Button>
              </div>
                </form>
              )}
            </CardContent>
          </Card>
          {isEditingChannel ? (
            <div className="flex justify-center">
              <AddNewButton
                id="settings-notification-channel-create"
                icon={Plus}
                label={t("settings.notificationChannelCreateNew")}
                onClick={startCreateChannel}
                disabled={mutatingChannelId !== null}
              />
            </div>
          ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-notification-channel-create"
            icon={Plus}
            label={t("settings.notificationChannelCreateNew")}
            onClick={startCreateChannel}
          />
        </div>
      )}

      {/* ── Subscriptions ─────────────────────────────── */}
      <CardTitle className="flex items-center gap-2 text-base">
        <Bell className="h-4 w-4" />
        {t("settings.notificationSubscriptions")}
      </CardTitle>

      <div className="rounded border border-border">
        <div className="overflow-x-auto">
          <Table id="settings-notification-subscriptions-table">
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.notificationEventType")}</TableHead>
                <TableHead>{t("settings.notificationChannel")}</TableHead>
                <TableHead>{t("settings.notificationScope")}</TableHead>
                <TableHead className="text-center">{t("label.enabled")}</TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {subscriptions.map((sub) => (
                <TableRow
                  data-ui="settings-table-row"
                  key={sub.id}
                  id={selectorId(
                    "settings-notification-subscription-row",
                    notificationTargetName(notificationTargets, sub.targetKind, sub.targetId),
                    orderedSubscriptionEventTypes(sub.eventTypes).join("-"),
                    sub.scope,
                    sub.scopeId,
                  )}
                  data-notification-target-name={notificationTargetName(
                    notificationTargets,
                    sub.targetKind,
                    sub.targetId,
                  )}
                  data-notification-target-kind={sub.targetKind}
                  data-notification-events={orderedSubscriptionEventTypes(sub.eventTypes).join(",")}
                  data-notification-scope={sub.scope}
                  data-notification-scope-id={sub.scopeId ?? ""}
                >
                  <TableCell>
                    <div className="space-y-1">
                      {orderedSubscriptionEventTypes(sub.eventTypes).map((eventType) => (
                        <div key={eventType} className="leading-snug">
                          {notificationEventLabel(eventType, t)}
                        </div>
                      ))}
                    </div>
                  </TableCell>
                  <TableCell>
                    {notificationTargetName(notificationTargets, sub.targetKind, sub.targetId)}
                  </TableCell>
                  <TableCell>
                    {(() => {
                      const scopeIdLabel = notificationScopeIdLabel(
                        sub.scope,
                        sub.scopeId,
                        t,
                        subscriptionTitlesById,
                      );

                      if (sub.scope !== "title" || !sub.scopeId) {
                        return (
                          <>
                            {notificationScopeLabel(sub.scope, t)}
                            {scopeIdLabel ? ` (${scopeIdLabel})` : ""}
                          </>
                        );
                      }

                      const title = subscriptionTitlesById[sub.scopeId];

                      return (
                        <>
                          {notificationScopeLabel(sub.scope, t)}
                          {" ("}
                          {title ? (
                            <Link
                              to={titleOverviewHref(title)}
                              className="underline-offset-4 hover:text-foreground hover:underline"
                            >
                              {formatResolvedTitleLabel(title)}
                            </Link>
                          ) : (
                            scopeIdLabel ?? t("label.unknown")
                          )}
                          {")"}
                        </>
                      );
                    })()}
                  </TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={sub.isEnabled}
                      label={`${t("label.enabled")}: ${orderedSubscriptionEventTypes(sub.eventTypes)
                        .map((eventType) => notificationEventLabel(eventType, t))
                        .join(", ")}`}
                    />
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="inline-flex items-center gap-2">
                      <NotificationActionButton
                        id={selectorId("settings-notification-subscription-toggle", sub.id)}
                        label={sub.isEnabled ? t("label.disable") : t("label.enable")}
                        tone={sub.isEnabled ? "enabled" : "disabled"}
                        onClick={() => void toggleSubscriptionEnabled(sub)}
                        disabled={mutatingSubscriptionId === sub.id}
                      >
                        {sub.isEnabled ? (
                          <Power className="h-4 w-4" />
                        ) : (
                          <PowerOff className="h-4 w-4" />
                        )}
                      </NotificationActionButton>
                      <NotificationActionButton
                        id={selectorId("settings-notification-subscription-edit", sub.id)}
                        label={t("label.edit")}
                        tone="edit"
                        onClick={() => editSubscription(sub)}
                      >
                        <Edit className="h-4 w-4" />
                      </NotificationActionButton>
                      <NotificationActionButton
                        id={selectorId("settings-notification-subscription-delete", sub.id)}
                        label={t("label.delete")}
                        tone="delete"
                        onClick={() => void deleteSubscription(sub)}
                        disabled={mutatingSubscriptionId === sub.id}
                      >
                        {mutatingSubscriptionId === sub.id ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <Trash2 className="h-4 w-4" />
                        )}
                      </NotificationActionButton>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
              {subscriptions.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-muted-foreground">
                    {t("settings.notificationNoSubscriptions")}
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </div>

      {isSubscriptionEditorOpen ? (
        <>
          <Card>
            <CardHeader>
              <CardTitle className="text-base">
                {isEditingSubscription
                  ? t("settings.notificationSubscriptionUpdate")
                  : t("settings.notificationSubscriptionCreate")}
              </CardTitle>
            </CardHeader>
            <CardContent>
              {notificationTargets.length === 0 ? (
                <p className="text-sm text-muted-foreground">{t("settings.notificationNoChannels")}</p>
              ) : (
                <form
                  id="settings-notification-subscription-form"
                  className="space-y-3"
                  onSubmit={submitSubscription}
                >
              <div className="grid gap-3 md:grid-cols-3">
                <label>
                  <Label className="mb-2 block">{t("settings.notificationEventType")}</Label>
                  <MultiSelectDropdown
                    options={eventTypeOptions}
                    selectedValues={subscriptionDraft.eventTypes}
                    onSelectedValuesChange={(values) =>
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        eventTypes: values,
                      }))
                    }
                    triggerLabel={
                      eventTypeOptions
                        .filter((option) => subscriptionDraft.eventTypes.includes(option.value))
                        .map((option) => option.label)
                        .join(", ") || t("settings.notificationEventType")
                    }
                    placeholder={t("settings.notificationEventType")}
                    id="settings-notification-subscription-event-types"
                    optionIdPrefix="settings-notification-subscription-event-type-option"
                  />
                </label>
                <label>
                  <Label className="mb-2 block">{t("settings.notificationChannel")}</Label>
                  <Select
                    value={
                      subscriptionDraft.targetId
                        ? `${subscriptionDraft.targetKind}:${subscriptionDraft.targetId}`
                        : undefined
                    }
                    onValueChange={(v) => {
                      const target = parseNotificationTargetValue(v);
                      if (!target) {
                        return;
                      }
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        targetKind: target.targetKind,
                        targetId: target.id,
                      }));
                    }}
                  >
                    <SelectTrigger id="settings-notification-subscription-target" className="w-full">
                      <SelectValue placeholder={t("settings.notificationChannel")} />
                    </SelectTrigger>
                    <SelectContent>
                      {notificationTargets
                        .filter((target) => target.isEnabled)
                        .map((target) => (
                          <SelectItem
                            id={selectorId(
                              "settings-notification-subscription-target-option",
                              target.name,
                            )}
                            key={notificationTargetValue(target)}
                            value={notificationTargetValue(target)}
                          >
                            {target.name}
                          </SelectItem>
                        ))}
                    </SelectContent>
                  </Select>
                </label>
                <label>
                  <Label className="mb-2 block">{t("settings.notificationScope")}</Label>
                  <Select
                    value={subscriptionDraft.scope || undefined}
                    onValueChange={(value) =>
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        scope: value,
                      }))
                    }
                  >
                    <SelectTrigger id="settings-notification-subscription-scope" className="w-full">
                      <SelectValue placeholder={t("settings.notificationScope")} />
                    </SelectTrigger>
                    <SelectContent>
                      {scopeOptions.map((option) => (
                        <SelectItem
                          id={selectorId("settings-notification-subscription-scope-option", option.value)}
                          key={option.value}
                          value={option.value}
                        >
                          {option.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </label>
              </div>

              {selectedSubscriptionProvider?.providerType === "jellyfin" ? (
                <p className="text-sm text-muted-foreground">
                  {t("settings.notificationJellyfinSubscriptionHint")}
                </p>
              ) : null}

              {subscriptionDraft.scope === "facet" ? (
                <div>
                  <Label className="mb-2 block">{t("settings.notificationScopeId")}</Label>
                  <div className="flex flex-wrap gap-4 rounded-md border border-border bg-card/40 p-3">
                    {FACET_SCOPE_OPTIONS.map((facet) => (
                      <label key={facet} className="flex items-center gap-2 text-sm">
                        <Checkbox
                          checked={selectedFacetScopeIds.includes(facet)}
                          onCheckedChange={(checked) =>
                            handleFacetScopeCheckedChange(facet, checked === true)
                          }
                        />
                        {sectionLabelForFacet(t, facet)}
                      </label>
                    ))}
                  </div>
                </div>
              ) : null}

              {subscriptionDraft.scope === "title" ? (
                <label>
                  <Label className="mb-2 block">{t("settings.notificationScopeId")}</Label>
                  <TitleAutocompletePicker
                    selectedTitle={subscriptionDraft.titleScopeTitle}
                    selectedTitleId={subscriptionDraft.titleScopeId || null}
                    onSelectedTitleChange={(title) =>
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        titleScopeId: title?.id ?? "",
                        titleScopeTitle: title,
                      }))
                    }
                    placeholder={t("settings.notificationScopeIdPlaceholderTitle")}
                    ariaLabel={t("settings.notificationScopeId")}
                  />
                </label>
              ) : null}

              <label className="flex items-center gap-2">
                <Checkbox
                  id="settings-notification-subscription-enabled"
                  checked={subscriptionDraft.isEnabled}
                  onCheckedChange={(value) =>
                    setSubscriptionDraft((prev) => ({
                      ...prev,
                      isEnabled: value === true,
                    }))
                  }
                />
                <span className="text-sm">{t("label.enabled")}</span>
              </label>

              <div className="flex gap-2">
                <Button
                  id="settings-notification-subscription-save"
                  type="submit"
                  disabled={!isSubscriptionDraftValid || mutatingSubscriptionId !== null}
                >
                  {mutatingSubscriptionId !== null
                    ? t("label.saving")
                    : editingSubscriptionId
                      ? t("settings.notificationSubscriptionUpdate")
                      : t("settings.notificationSubscriptionCreate")}
                </Button>
                <Button
                  id="settings-notification-subscription-cancel"
                  type="button"
                  variant="outline"
                  onClick={resetSubscriptionDraft}
                >
                  {t("label.cancel")}
                </Button>
              </div>
                </form>
              )}
            </CardContent>
          </Card>
          {isEditingSubscription ? (
            <div className="flex justify-center">
              <AddNewButton
                id="settings-notification-subscription-create"
                icon={Plus}
                label={t("settings.notificationSubscriptionCreateNew")}
                onClick={() => startCreateSubscription()}
                disabled={mutatingSubscriptionId !== null}
              />
            </div>
          ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-notification-subscription-create"
            icon={Plus}
            label={t("settings.notificationSubscriptionCreateNew")}
            onClick={() => startCreateSubscription()}
          />
        </div>
      )}
    </div>
  );
}

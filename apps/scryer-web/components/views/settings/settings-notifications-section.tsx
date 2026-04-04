
import * as React from "react";
import { Bell, Edit, Power, PowerOff, Send, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input, signedIntegerInputProps } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
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
import type { Translate } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  ConfigFieldDef,
  NotificationChannel,
  NotificationChannelDraft,
  NotificationProviderType,
  NotificationSubscription,
  NotificationSubscriptionDraft,
} from "@/lib/types";

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
  providerTypes: NotificationProviderType[];
  subscriptions: NotificationSubscription[];
  editingSubscriptionId: string | null;
  subscriptionDraft: NotificationSubscriptionDraft;
  setSubscriptionDraft: React.Dispatch<React.SetStateAction<NotificationSubscriptionDraft>>;
  submitSubscription: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mutatingSubscriptionId: string | null;
  resetSubscriptionDraft: () => void;
  editSubscription: (sub: NotificationSubscription) => void;
  toggleSubscriptionEnabled: (sub: NotificationSubscription) => Promise<void> | void;
  deleteSubscription: (sub: NotificationSubscription) => Promise<void> | void;
  eventTypes: string[];
};

const SCOPE_OPTIONS = ["global", "facet", "title"] as const;

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

function DynamicConfigField({
  field,
  value,
  onChange,
}: {
  field: ConfigFieldDef;
  value: string;
  onChange: (key: string, value: string) => void;
}) {
  if (field.fieldType === "bool") {
    return (
      <label className="flex items-center gap-2">
        <input
          type="checkbox"
          checked={value === "true"}
          onChange={(e) => onChange(field.key, e.target.checked ? "true" : "false")}
          className="accent-primary"
        />
        <span className="text-sm">{field.label}</span>
        {field.helpText ? (
          <span className="text-xs text-muted-foreground">{field.helpText}</span>
        ) : null}
      </label>
    );
  }

  if (field.fieldType === "select" && field.options.length > 0) {
    return (
      <label>
        <Label className="mb-2 block">{field.label}</Label>
        <Select
          value={value || field.defaultValue || ""}
          onValueChange={(v) => onChange(field.key, v)}
        >
          <SelectTrigger className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {field.options.map((opt) => (
              <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
            ))}
          </SelectContent>
        </Select>
        {field.helpText ? (
          <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
        ) : null}
      </label>
    );
  }

  if (field.fieldType === "multiline") {
    return (
      <label>
        <Label className="mb-2 block">{field.label}</Label>
        <Textarea
          value={value}
          onChange={(e) => onChange(field.key, e.target.value)}
          required={field.required}
          placeholder={field.defaultValue ?? ""}
          rows={6}
        />
        {field.helpText ? (
          <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
        ) : null}
      </label>
    );
  }

  return (
    <label>
      <Label className="mb-2 block">{field.label}</Label>
      <Input
        value={value}
        onChange={(e) => onChange(field.key, e.target.value)}
        {...(field.fieldType === "number" ? signedIntegerInputProps : {})}
        type={
          field.fieldType === "password" || field.fieldType === "secret"
            ? "password"
            : field.fieldType === "number"
              ? "number"
              : "text"
        }
        required={field.required}
        placeholder={field.defaultValue ?? ""}
      />
      {field.helpText ? (
        <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
      ) : null}
    </label>
  );
}

function channelNameById(channels: NotificationChannel[], id: string): string {
  return channels.find((c) => c.id === id)?.name ?? id;
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
  providerTypes,
  subscriptions,
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
}: SettingsNotificationsSectionProps) {
  const t = useTranslate();
  const normalizedChannelType = channelDraft.channelType.trim().toLowerCase();

  const providerTypeOptions = React.useMemo(() => {
    if (providerTypes.length === 0) return [];
    return providerTypes.map((pt) => ({ value: pt.providerType, label: pt.name }));
  }, [providerTypes]);

  const selectedProvider = React.useMemo(() => {
    return providerTypes.find(
      (pt) => pt.providerType === normalizedChannelType,
    ) ?? null;
  }, [normalizedChannelType, providerTypes]);

  const selectedProviderFields = selectedProvider?.configFields ?? [];

  const handleConfigValueChange = React.useCallback(
    (key: string, value: string) => {
      setChannelDraft((prev) => ({
        ...prev,
        configValues: { ...prev.configValues, [key]: value },
      }));
    },
    [setChannelDraft],
  );

  return (
    <div className="space-y-6 text-sm">
      {/* ── Channels ──────────────────────────────────── */}
      <CardTitle className="flex items-center gap-2 text-base">
        <Bell className="h-4 w-4" />
        {t("settings.notificationChannels")}
      </CardTitle>

      <div className="rounded border border-border">
        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead>{t("settings.notificationProviderType")}</TableHead>
                <TableHead className="text-center">{t("label.enabled")}</TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {channels.map((channel) => (
                <TableRow key={channel.id}>
                  <TableCell>{channel.name}</TableCell>
                  <TableCell>{channel.channelType}</TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={channel.isEnabled}
                      label={`${t("label.enabled")}: ${channel.name}`}
                    />
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => void testChannel(channel)}
                        disabled={testingChannelId === channel.id}
                      >
                        <Send className="mr-1 h-3.5 w-3.5" />
                        {testingChannelId === channel.id ? t("settings.notificationTesting") : t("settings.notificationTest")}
                      </Button>
                      <Button
                        size="icon"
                        variant="ghost"
                        onClick={() => void toggleChannelEnabled(channel)}
                        disabled={mutatingChannelId === channel.id}
                        title={channel.isEnabled ? t("label.disable") : t("label.enable")}
                      >
                        {channel.isEnabled ? (
                          <Power className="h-4 w-4 text-green-400" />
                        ) : (
                          <PowerOff className="h-4 w-4 text-red-400" />
                        )}
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => editChannel(channel)}
                      >
                        <Edit className="mr-1 h-3.5 w-3.5" />
                        {t("label.update")}
                      </Button>
                      <Button
                        size="sm"
                        variant="destructive"
                        onClick={() => void deleteChannel(channel)}
                        disabled={mutatingChannelId === channel.id}
                      >
                        <Trash2 className="mr-1 h-3.5 w-3.5" />
                        {mutatingChannelId === channel.id ? t("label.deleting") : t("label.delete")}
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
              {channels.length === 0 ? (
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

      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {editingChannelId ? t("settings.notificationChannelUpdate") : t("settings.notificationChannelCreate")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {providerTypeOptions.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("settings.notificationNoProviders")}</p>
          ) : (
            <form className="space-y-3" onSubmit={submitChannel}>
              <div className="grid gap-3 md:grid-cols-2">
                <label>
                  <Label className="mb-2 block">{t("label.name")}</Label>
                  <Input
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
                <label>
                  <Label className="mb-2 block">{t("settings.notificationProviderType")}</Label>
                  <Select
                    value={normalizedChannelType || undefined}
                    onValueChange={(v) =>
                      setChannelDraft((prev) => ({
                        ...prev,
                        channelType: v,
                      }))
                    }
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue placeholder={t("settings.notificationProviderType")} />
                    </SelectTrigger>
                    <SelectContent>
                      {providerTypeOptions.map((opt) => (
                        <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </label>
              </div>

              {selectedProviderFields.length > 0 ? (
                <div className="space-y-3">
                  <div className="grid gap-3 md:grid-cols-2">
                    {selectedProviderFields
                      .filter((f) => f.fieldType !== "bool")
                      .map((field) => (
                        <DynamicConfigField
                          key={field.key}
                          field={field}
                          value={channelDraft.configValues[field.key] ?? field.defaultValue ?? ""}
                          onChange={handleConfigValueChange}
                        />
                      ))}
                  </div>
                  {selectedProviderFields.some((f) => f.fieldType === "bool") ? (
                    <div className="flex items-center gap-6">
                      {selectedProviderFields
                        .filter((f) => f.fieldType === "bool")
                        .map((field) => (
                          <DynamicConfigField
                            key={field.key}
                            field={field}
                            value={channelDraft.configValues[field.key] ?? field.defaultValue ?? "false"}
                            onChange={handleConfigValueChange}
                          />
                        ))}
                    </div>
                  ) : null}
                </div>
              ) : null}

              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={channelDraft.isEnabled}
                  onChange={(event) =>
                    setChannelDraft((prev) => ({
                      ...prev,
                      isEnabled: event.target.checked,
                    }))
                  }
                  className="accent-primary"
                />
                <span className="text-sm">{t("label.enabled")}</span>
              </label>

              <div className="flex gap-2">
                <Button type="submit" disabled={mutatingChannelId === "new"}>
                  {mutatingChannelId === "new"
                    ? t("label.saving")
                    : editingChannelId
                      ? t("settings.notificationChannelUpdate")
                      : t("settings.notificationChannelCreate")}
                </Button>
                <Button type="button" variant="secondary" onClick={resetChannelDraft}>
                  {t("label.cancel")}
                </Button>
              </div>
            </form>
          )}
        </CardContent>
      </Card>

      {/* ── Subscriptions ─────────────────────────────── */}
      <CardTitle className="flex items-center gap-2 text-base">
        <Bell className="h-4 w-4" />
        {t("settings.notificationSubscriptions")}
      </CardTitle>

      <div className="rounded border border-border">
        <div className="overflow-x-auto">
          <Table>
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
                <TableRow key={sub.id}>
                  <TableCell>{notificationEventLabel(sub.eventType, t)}</TableCell>
                  <TableCell>{channelNameById(channels, sub.channelId)}</TableCell>
                  <TableCell>
                    {notificationScopeLabel(sub.scope, t)}
                    {sub.scopeId ? ` (${sub.scopeId})` : ""}
                  </TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={sub.isEnabled}
                      label={`${t("label.enabled")}: ${notificationEventLabel(sub.eventType, t)}`}
                    />
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-2">
                      <Button
                        size="icon"
                        variant="ghost"
                        onClick={() => void toggleSubscriptionEnabled(sub)}
                        disabled={mutatingSubscriptionId === sub.id}
                        title={sub.isEnabled ? t("label.disable") : t("label.enable")}
                      >
                        {sub.isEnabled ? (
                          <Power className="h-4 w-4 text-green-400" />
                        ) : (
                          <PowerOff className="h-4 w-4 text-red-400" />
                        )}
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => editSubscription(sub)}
                      >
                        <Edit className="mr-1 h-3.5 w-3.5" />
                        {t("label.update")}
                      </Button>
                      <Button
                        size="sm"
                        variant="destructive"
                        onClick={() => void deleteSubscription(sub)}
                        disabled={mutatingSubscriptionId === sub.id}
                      >
                        <Trash2 className="mr-1 h-3.5 w-3.5" />
                        {mutatingSubscriptionId === sub.id ? t("label.deleting") : t("label.delete")}
                      </Button>
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

      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {editingSubscriptionId ? t("settings.notificationSubscriptionUpdate") : t("settings.notificationSubscriptionCreate")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {channels.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t("settings.notificationNoChannels")}</p>
          ) : (
            <form className="space-y-3" onSubmit={submitSubscription}>
              <div className="grid gap-3 md:grid-cols-3">
                <label>
                  <Label className="mb-2 block">{t("settings.notificationEventType")}</Label>
                  <Select
                    value={subscriptionDraft.eventType || undefined}
                    onValueChange={(v) =>
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        eventType: v,
                      }))
                    }
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue placeholder={t("settings.notificationEventType")} />
                    </SelectTrigger>
                    <SelectContent>
                      {eventTypes.map((et) => (
                        <SelectItem key={et} value={et}>
                          {notificationEventLabel(et, t)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </label>
                <label>
                  <Label className="mb-2 block">{t("settings.notificationChannel")}</Label>
                  <Select
                    value={subscriptionDraft.channelId || undefined}
                    onValueChange={(v) =>
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        channelId: v,
                      }))
                    }
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue placeholder={t("settings.notificationChannel")} />
                    </SelectTrigger>
                    <SelectContent>
                      {channels.map((ch) => (
                        <SelectItem key={ch.id} value={ch.id}>{ch.name}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </label>
                <label>
                  <Label className="mb-2 block">{t("settings.notificationScope")}</Label>
                  <Select
                    value={subscriptionDraft.scope}
                    onValueChange={(v) =>
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        scope: v,
                      }))
                    }
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {SCOPE_OPTIONS.map((scope) => (
                        <SelectItem key={scope} value={scope}>
                          {notificationScopeLabel(scope, t)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </label>
              </div>

              {subscriptionDraft.scope !== "global" ? (
                <label>
                  <Label className="mb-2 block">{t("settings.notificationScopeId")}</Label>
                  <Input
                    value={subscriptionDraft.scopeId}
                    onChange={(event) =>
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        scopeId: event.target.value,
                      }))
                    }
                    placeholder={
                      subscriptionDraft.scope === "facet"
                        ? t("settings.notificationScopeIdPlaceholderFacet")
                        : t("settings.notificationScopeIdPlaceholderTitle")
                    }
                  />
                </label>
              ) : null}

              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={subscriptionDraft.isEnabled}
                  onChange={(event) =>
                    setSubscriptionDraft((prev) => ({
                      ...prev,
                      isEnabled: event.target.checked,
                    }))
                  }
                  className="accent-primary"
                />
                <span className="text-sm">{t("label.enabled")}</span>
              </label>

              <div className="flex gap-2">
                <Button type="submit" disabled={mutatingSubscriptionId === "new"}>
                  {mutatingSubscriptionId === "new"
                    ? t("label.saving")
                    : editingSubscriptionId
                      ? t("settings.notificationSubscriptionUpdate")
                      : t("settings.notificationSubscriptionCreate")}
                </Button>
                <Button type="button" variant="secondary" onClick={resetSubscriptionDraft}>
                  {t("label.cancel")}
                </Button>
              </div>
            </form>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

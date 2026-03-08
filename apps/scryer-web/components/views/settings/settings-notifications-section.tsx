
import * as React from "react";
import { Bell, Edit, Power, PowerOff, Send, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
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
import type {
  NotificationChannel,
  NotificationChannelDraft,
  NotificationSubscription,
  NotificationSubscriptionDraft,
  NotificationProviderType,
  ConfigFieldDef,
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

const SCOPE_OPTIONS = [
  { value: "global", label: "Global" },
  { value: "facet", label: "Facet" },
  { value: "title", label: "Title" },
];

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

  return (
    <label>
      <Label className="mb-2 block">{field.label}</Label>
      <Input
        value={value}
        onChange={(e) => onChange(field.key, e.target.value)}
        type={field.fieldType === "password" ? "password" : field.fieldType === "number" ? "number" : "text"}
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
                  <TableCell>{sub.eventType}</TableCell>
                  <TableCell>{channelNameById(channels, sub.channelId)}</TableCell>
                  <TableCell>
                    {sub.scope}
                    {sub.scopeId ? ` (${sub.scopeId})` : ""}
                  </TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={sub.isEnabled}
                      label={`${t("label.enabled")}: ${sub.eventType}`}
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
                        <SelectItem key={et} value={et}>{et}</SelectItem>
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
                      {SCOPE_OPTIONS.map((opt) => (
                        <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </label>
              </div>

              {subscriptionDraft.scope !== "global" ? (
                <label>
                  <Label className="mb-2 block">Scope ID</Label>
                  <Input
                    value={subscriptionDraft.scopeId}
                    onChange={(event) =>
                      setSubscriptionDraft((prev) => ({
                        ...prev,
                        scopeId: event.target.value,
                      }))
                    }
                    placeholder={subscriptionDraft.scope === "facet" ? "movie, tv, anime" : "Title ID"}
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

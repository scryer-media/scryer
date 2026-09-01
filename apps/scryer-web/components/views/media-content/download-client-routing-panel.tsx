import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { IconButton } from "@/components/ui/icon-button";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { DownloadClientTypeLogo } from "@/components/common/download-client-type-logo";
import { Checkbox } from "@/components/ui/checkbox";
import { ChevronDown, ChevronUp, Download, Power, PowerOff } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { DownloadClientRecord, DownloadClientRoutingSettings } from "@/lib/types";
import { selectorId } from "@/lib/utils/dom-ids";
import { DOWNLOAD_CLIENT_ROUTING_EMPTY } from "@/lib/constants/nzbget";
import {
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";
import { useSeedingProfileOptions } from "@/lib/hooks/use-seeding-profile-options";
import {
  SEEDING_PROFILE_INHERIT_VALUE,
  seedingProfileSelectValue,
  seedingProfileSelectValueToId,
} from "@/lib/utils/seeding-profiles";

type ScopeRoutingRecord = Record<string, DownloadClientRoutingSettings>;

function DownloadClientRoutingActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<typeof IconButton>, "tone"> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "enabled" | "disabled" | "reorder">;
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

export const DOWNLOAD_PRIORITY_OPTIONS = [
  { value: "force", label: "settings.downloadClientPriorityForce" },
  { value: "very high", label: "settings.downloadClientPriorityVeryHigh" },
  { value: "high", label: "settings.downloadClientPriorityHigh" },
  { value: "normal", label: "settings.downloadClientPriorityNormal" },
  { value: "low", label: "settings.downloadClientPriorityLow" },
  { value: "very low", label: "settings.downloadClientPriorityVeryLow" },
];

const PRIORITY_VALUES = new Set(DOWNLOAD_PRIORITY_OPTIONS.map((item) => item.value));

function normalizePriorityValue(rawValue: string): string {
  const normalized = rawValue.trim().toLowerCase();
  if (!normalized) {
    return "normal";
  }

  if (PRIORITY_VALUES.has(normalized)) {
    return normalized;
  }

  const aliased = normalized.replace(/_/g, " ");
  return PRIORITY_VALUES.has(aliased) ? aliased : "normal";
}

function normalizePriorityValueForSave(rawValue: string): string {
  const normalized = rawValue.trim().toLowerCase();
  if (!normalized) {
    return "normal";
  }

  if (PRIORITY_VALUES.has(normalized)) {
    return normalized;
  }

  const aliased = normalized.replace(/_/g, " ");
  return PRIORITY_VALUES.has(aliased) ? aliased : "normal";
}

type DownloadClientRoutingPanelProps = {
  scopeLabel: string;
  downloadClients: DownloadClientRecord[];
  activeScopeRouting: ScopeRoutingRecord;
  activeScopeRoutingOrder: string[];
  downloadClientRoutingLoading: boolean;
  downloadClientRoutingSaving: boolean;
  updateDownloadClientRoutingForScope: (
    clientId: string,
    nextValue: Partial<DownloadClientRoutingSettings>,
    options?: { save?: boolean },
  ) => Promise<void> | void;
  moveDownloadClientInScope: (clientId: string, direction: "up" | "down") => void;
};

export const DownloadClientRoutingPanel = React.memo(function DownloadClientRoutingPanel({
  scopeLabel,
  downloadClients,
  activeScopeRouting,
  activeScopeRoutingOrder,
  downloadClientRoutingLoading,
  downloadClientRoutingSaving,
  updateDownloadClientRoutingForScope,
  moveDownloadClientInScope,
}: DownloadClientRoutingPanelProps) {
  const t = useTranslate();
  // Seeding profiles are reference data for this table, not scope state, so the
  // panel reads them itself instead of threading an identical prop through the
  // two unrelated parents that render it.
  const { options: seedingProfileOptions } = useSeedingProfileOptions();
  const clientById = React.useMemo(
    () => Object.fromEntries(downloadClients.map((client) => [client.id, client])),
    [downloadClients],
  );

  const orderedDownloadClientIds = React.useMemo(() => {
    const configuredIds = activeScopeRoutingOrder.filter((clientId) => clientById[clientId]);
    const configuredIdSet = new Set(configuredIds);
    const missingIds = downloadClients
      .map((client) => client.id)
      .filter((clientId) => !configuredIdSet.has(clientId));
    return [...configuredIds, ...missingIds];
  }, [activeScopeRoutingOrder, clientById, downloadClients]);

  const handleRoutingCategoryChange = React.useCallback(
    (clientId: string, value: string) => {
      void updateDownloadClientRoutingForScope(
        clientId,
        {
          category: value,
        },
        { save: false },
      );
    },
    [updateDownloadClientRoutingForScope],
  );

  const handleRoutingCategoryBlur = React.useCallback(
    (clientId: string, value: string) => {
      void updateDownloadClientRoutingForScope(clientId, {
        category: value,
      });
    },
    [updateDownloadClientRoutingForScope],
  );

  const handleRoutingRecentPriorityChange = React.useCallback(
    (clientId: string, value: string) => {
      void updateDownloadClientRoutingForScope(clientId, {
        recentQueuePriority: normalizePriorityValueForSave(value),
      });
    },
    [updateDownloadClientRoutingForScope],
  );

  const handleRoutingOlderPriorityChange = React.useCallback(
    (clientId: string, value: string) => {
      void updateDownloadClientRoutingForScope(clientId, {
        olderQueuePriority: normalizePriorityValueForSave(value),
      });
    },
    [updateDownloadClientRoutingForScope],
  );

  const handleRoutingRemoveCompletedChange = React.useCallback(
    (clientId: string, checked: boolean) => {
      void updateDownloadClientRoutingForScope(clientId, {
        removeCompleted: checked,
      });
    },
    [updateDownloadClientRoutingForScope],
  );

  const handleRoutingRemoveFailedChange = React.useCallback(
    (clientId: string, checked: boolean) => {
      void updateDownloadClientRoutingForScope(clientId, {
        removeFailed: checked,
      });
    },
    [updateDownloadClientRoutingForScope],
  );

  const handleRoutingSeedingProfileChange = React.useCallback(
    (clientId: string, value: string) => {
      void updateDownloadClientRoutingForScope(clientId, {
        seedingProfileId: seedingProfileSelectValueToId(value),
      });
    },
    [updateDownloadClientRoutingForScope],
  );

  const moveClientUp = React.useCallback(
    (clientId: string) => {
      moveDownloadClientInScope(clientId, "up");
    },
    [moveDownloadClientInScope],
  );

  const moveClientDown = React.useCallback(
    (clientId: string) => {
      moveDownloadClientInScope(clientId, "down");
    },
    [moveDownloadClientInScope],
  );

  return (
    <section
      id="download-client-routing-panel"
      className="rounded-[16px] border border-[var(--scry-border)] bg-[var(--scry-surf)] p-5 sm:p-6"
    >
      <div className="flex items-center gap-2.5">
        <Download className="h-[17px] w-[17px] text-[var(--scry-accent-text)]" />
        <h2 className="text-[16px] font-bold text-[var(--scry-ink2)]">
          {t("settings.downloadClientRoutingScope", {
            scope: scopeLabel,
          })}
        </h2>
      </div>
      <div className="mt-5">
        <div className="overflow-x-auto rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-card2)]">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.downloadClientPriority")}</TableHead>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead>{t("label.type")}</TableHead>
                <TableHead className="text-center">{t("settings.downloadClientRoutingGloballyEnabled")}</TableHead>
                <TableHead className="text-center">{t("settings.downloadClientRoutingEnabled")}</TableHead>
                <TableHead>{t("settings.downloadClientCategory")}</TableHead>
                <TableHead>{t("settings.downloadClientRecentPriority")}</TableHead>
                <TableHead>{t("settings.downloadClientOlderPriority")}</TableHead>
                <TableHead className="text-center">{t("settings.downloadClientRemoveCompleted")}</TableHead>
                <TableHead className="text-center">{t("settings.downloadClientRemoveFailed")}</TableHead>
                <TableHead>{t("settings.seedingProfileColumn")}</TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {orderedDownloadClientIds.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={12} className="text-muted-foreground">
                    {t("settings.noDownloadClientsFound")}
                  </TableCell>
                </TableRow>
              ) : (
                orderedDownloadClientIds.map((clientId, index) => {
                  const client = clientById[clientId];
                  if (!client) {
                    return null;
                  }
                  const routing =
                    activeScopeRouting[client.id] ?? DOWNLOAD_CLIENT_ROUTING_EMPTY;
                  const controlsDisabled =
                    downloadClientRoutingLoading || downloadClientRoutingSaving;

                  return (
                    <TableRow
                      key={client.id}
                      id={selectorId("download-client-routing-row", client.name)}
                      data-ui="settings-table-row"
                    >
                      <TableCell>{index + 1}</TableCell>
                      <TableCell>{client.name}</TableCell>
                      <TableCell className="text-center">
                        <span className="inline-flex items-center justify-center">
                          <DownloadClientTypeLogo typeValue={client.clientType} />
                          <span className="sr-only">{client.clientType}</span>
                        </span>
                      </TableCell>
                      <TableCell className="text-center align-middle">
                        <RenderBooleanIcon
                          value={client.isEnabled}
                          label={`${t("settings.downloadClientRoutingGloballyEnabled")}: ${client.name}`}
                        />
                      </TableCell>
                      <TableCell className="text-center align-middle">
                        <RenderBooleanIcon
                          value={client.isEnabled && routing.enabled}
                          label={`${t("settings.downloadClientRoutingEnabled")}: ${client.name}`}
                        />
                      </TableCell>
                      <TableCell>
                        <Input
                          id={selectorId(
                            "download-client-routing-category",
                            client.name,
                          )}
                          value={routing.category}
                          onChange={(event) =>
                            handleRoutingCategoryChange(client.id, event.target.value)
                          }
                          onBlur={(event) =>
                            handleRoutingCategoryBlur(client.id, event.target.value)
                          }
                          disabled={controlsDisabled}
                          placeholder={t("settings.downloadClientCategoryPlaceholder")}
                        />
                      </TableCell>
                      <TableCell>
                        <Select
                          value={normalizePriorityValue(routing.recentQueuePriority)}
                          onValueChange={(value) =>
                            handleRoutingRecentPriorityChange(client.id, value)
                          }
                          disabled={controlsDisabled}
                        >
                          <SelectTrigger
                            id={selectorId(
                              "download-client-routing-recent-priority",
                              client.name,
                            )}
                            className="w-full"
                          >
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {DOWNLOAD_PRIORITY_OPTIONS.map((option) => (
                              <SelectItem key={option.value} value={option.value}>
                                {t(option.label)}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </TableCell>
                      <TableCell>
                        <Select
                          value={normalizePriorityValue(routing.olderQueuePriority)}
                          onValueChange={(value) =>
                            handleRoutingOlderPriorityChange(client.id, value)
                          }
                          disabled={controlsDisabled}
                        >
                          <SelectTrigger
                            id={selectorId(
                              "download-client-routing-older-priority",
                              client.name,
                            )}
                            className="w-full"
                          >
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {DOWNLOAD_PRIORITY_OPTIONS.map((option) => (
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
                            "download-client-routing-remove-completed",
                            client.name,
                          )}
                          checked={routing.removeCompleted}
                          onCheckedChange={(checked) =>
                            handleRoutingRemoveCompletedChange(client.id, checked === true)
                          }
                          disabled={controlsDisabled}
                        />
                      </TableCell>
                      <TableCell className="text-center">
                        <Checkbox
                          id={selectorId(
                            "download-client-routing-remove-failed",
                            client.name,
                          )}
                          checked={routing.removeFailed}
                          onCheckedChange={(checked) =>
                            handleRoutingRemoveFailedChange(client.id, checked === true)
                          }
                          disabled={controlsDisabled}
                        />
                      </TableCell>
                      <TableCell>
                        <Select
                          value={seedingProfileSelectValue(
                            routing.seedingProfileId,
                          )}
                          onValueChange={(value) =>
                            handleRoutingSeedingProfileChange(client.id, value)
                          }
                          disabled={controlsDisabled}
                        >
                          <SelectTrigger
                            id={selectorId(
                              "download-client-routing-seeding-profile",
                              client.name,
                            )}
                            className="w-full min-w-[180px]"
                            aria-label={t("settings.seedingProfileRoutingLabel", {
                              name: client.name,
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
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-2">
                          <DownloadClientRoutingActionButton
                            id={selectorId(
                              routing.enabled
                                ? "download-client-routing-disable"
                                : "download-client-routing-enable",
                              client.name,
                            )}
                            tone={routing.enabled ? "disabled" : "enabled"}
                            label={
                              routing.enabled
                                ? t("label.disable")
                                : t("label.enable")
                            }
                            onClick={() =>
                              void updateDownloadClientRoutingForScope(client.id, {
                                enabled: !routing.enabled,
                              })
                            }
                            disabled={controlsDisabled || !client.isEnabled}
                          >
                            {routing.enabled ? (
                              <PowerOff className="h-4 w-4" />
                            ) : (
                              <Power className="h-4 w-4" />
                            )}
                          </DownloadClientRoutingActionButton>
                          <DownloadClientRoutingActionButton
                            id={selectorId(
                              "download-client-routing-move-up",
                              client.name,
                            )}
                            tone="reorder"
                            label={`${t("label.moveUp")} ${client.name}`}
                            onClick={() => moveClientUp(client.id)}
                            disabled={controlsDisabled || index === 0}
                          >
                            <ChevronUp className="h-4 w-4" />
                          </DownloadClientRoutingActionButton>
                          <DownloadClientRoutingActionButton
                            id={selectorId(
                              "download-client-routing-move-down",
                              client.name,
                            )}
                            tone="reorder"
                            label={`${t("label.moveDown")} ${client.name}`}
                            onClick={() => moveClientDown(client.id)}
                            disabled={
                              controlsDisabled ||
                              index >= orderedDownloadClientIds.length - 1
                            }
                          >
                            <ChevronDown className="h-4 w-4" />
                          </DownloadClientRoutingActionButton>
                        </div>
                      </TableCell>
                    </TableRow>
                  );
                })
              )}
            </TableBody>
          </Table>
        </div>
      </div>
    </section>
  );
});

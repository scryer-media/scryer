import * as React from "react";
import { Link } from "react-router";
import { Check, ChevronsUpDown, Loader2, Plus, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Label } from "@/components/ui/label";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { isVisibleExternalAccountProvider } from "@/lib/constants/integration-providers";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type {
  ExternalAccountProvider,
  ExternalAuthRuntimeConnection,
  ExternalAuthRuntimeSettings,
  LinkedAccount,
  MediaServerUser,
  MediaServerUserGroup,
  UiDateTimeFormat,
} from "@/lib/types/settings";
import { cn } from "@/lib/utils";
import { AuthenticatedAvatar } from "@/components/common/authenticated-avatar";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { selectorId } from "@/lib/utils/dom-ids";

const EXTERNAL_INVITES_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const EXTERNAL_INVITES_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const EXTERNAL_INVITES_PANEL_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const EXTERNAL_INVITES_TABLE_SHELL_CLASS =
  "overflow-hidden rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]";
const EXTERNAL_INVITES_TABLE_HEADER_ROW_CLASS =
  "border-[var(--scry-border3)] bg-[var(--scry-inset)] hover:bg-[var(--scry-inset)]";
const EXTERNAL_INVITES_TABLE_HEADER_CELL_CLASS =
  "font-semibold text-[var(--scry-muted2)]";

export type ExternalInviteDraft = {
  userId: string;
  provider: ExternalAccountProvider;
  connectionId: string;
  providerUserIdentifier: string;
  providerUserId: string;
};

export type ExternalInviteUser = {
  id: string;
  username: string;
};

export type ExternalInviteMediaServerUserGroup = MediaServerUserGroup;

type ExternalInviteMediaServerUserOption = MediaServerUser & {
  provider: ExternalAccountProvider;
  connectionId: string;
  connectionName: string;
};

type ExternalAccountInvitesPanelProps = {
  users: ExternalInviteUser[];
  invites: LinkedAccount[];
  mediaServerUserGroups: ExternalInviteMediaServerUserGroup[];
  mediaServerUserSearchLoading: boolean;
  mediaServerUserLookupError: string | null;
  externalAuthSettings: ExternalAuthRuntimeSettings;
  loading: boolean;
  externalInviteDraft: ExternalInviteDraft;
  externalInviteSubmitting: boolean;
  updateExternalInviteDraft: (patch: Partial<ExternalInviteDraft>) => void;
  createExternalAccountInvite: (
    event: React.FormEvent<HTMLFormElement>,
  ) => Promise<void> | void;
  showMediaServersLink?: boolean;
};

function providerLabel(provider: ExternalAccountProvider): string {
  switch (provider) {
    case "PLEX":
      return "Plex";
    case "JELLYFIN":
      return "Jellyfin";
    case "EMBY":
      return "Emby";
    default:
      return provider;
  }
}

function providerConnections(
  settings: ExternalAuthRuntimeSettings,
  provider: ExternalAccountProvider,
): ExternalAuthRuntimeConnection[] {
  return settings.connections.filter(
    (connection) => connection.provider === provider && connection.loginEnabled,
  );
}

function providerConnectionLabel(
  connection: ExternalAuthRuntimeConnection,
): string {
  return connection.displayName;
}

function inviteConnectionLabel(
  settings: ExternalAuthRuntimeSettings,
  invite: LinkedAccount,
): string {
  const connection = providerConnections(settings, invite.provider).find(
    (candidate) => candidate.id === invite.connectionId,
  );
  if (connection) {
    return providerConnectionLabel(connection);
  }

  return invite.provider === "JELLYFIN"
    ? providerLabel(invite.provider)
    : invite.connectionId;
}

function formatTimestamp(
  value: string | null | undefined,
  dateTimeFormat: UiDateTimeFormat,
): string {
  return formatUiDateTime(value, dateTimeFormat, { fallback: "-" });
}

function providerIdentityLabel(account: LinkedAccount): string {
  return account.displayName || account.username;
}

function ProviderAvatar({
  avatarUrl,
  label,
}: {
  avatarUrl: string | null | undefined;
  label: string;
}) {
  return (
    <AuthenticatedAvatar
      avatarUrl={avatarUrl}
      label={label}
      imageClassName="h-7 w-7 shrink-0 rounded-full border border-border object-cover"
      fallbackClassName="flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-border bg-muted text-xs font-medium text-muted-foreground"
    />
  );
}

function mediaServerUserGroupLabel(
  group: ExternalInviteMediaServerUserGroup,
): string {
  return `${providerLabel(group.provider)} - ${group.connectionName}`;
}

function MediaServerUserCombobox({
  id,
  value,
  groups,
  selectedOption,
  loading,
  disabled,
  placeholder,
  emptyLabel,
  serverEmptyLabel,
  loadingLabel,
  onChange,
  onSelectOption,
}: {
  id: string;
  value: string;
  groups: ExternalInviteMediaServerUserGroup[];
  selectedOption: ExternalInviteMediaServerUserOption | null;
  loading: boolean;
  disabled: boolean;
  placeholder: string;
  emptyLabel: string;
  serverEmptyLabel: string;
  loadingLabel: string;
  onChange: (value: string) => void;
  onSelectOption: (option: ExternalInviteMediaServerUserOption) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const normalizedValue = value.trim().toLocaleLowerCase();
  const filteredGroups = React.useMemo(
    () =>
      groups
        .map((group) => {
          if (group.status !== "READY" || !normalizedValue) {
            return group;
          }

          return {
            ...group,
            users: group.users.filter((option) => {
              const searchable = [
                option.username,
                option.id,
                option.displayName ?? "",
                group.connectionName,
                providerLabel(group.provider),
              ]
                .join(" ")
                .toLocaleLowerCase();
              return searchable.includes(normalizedValue);
            }),
          };
        })
        .filter(
          (group) =>
            group.status !== "READY" ||
            group.users.length > 0 ||
            !normalizedValue,
        ),
    [groups, normalizedValue],
  );
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          id={id}
          type="button"
          variant="outline"
          role="combobox"
          aria-expanded={open}
          className="h-9 w-full justify-between border-input bg-field px-3 text-left font-normal shadow-xs hover:bg-field hover:text-foreground"
          disabled={disabled}
        >
          <span className="flex min-w-0 items-center gap-2">
            {selectedOption ? (
              <ProviderAvatar
                avatarUrl={selectedOption.avatarUrl}
                label={selectedOption.displayName ?? selectedOption.username}
              />
            ) : null}
            <span
              className={
                value.trim() ? "truncate" : "truncate text-muted-foreground"
              }
            >
              {selectedOption
                ? `${selectedOption.displayName ?? selectedOption.username} (${providerLabel(selectedOption.provider)} - ${selectedOption.connectionName})`
                : value.trim() || placeholder}
            </span>
          </span>
          {loading ? (
            <Loader2 className="h-4 w-4 shrink-0 animate-spin text-muted-foreground" />
          ) : (
            <ChevronsUpDown className="h-4 w-4 shrink-0 text-muted-foreground" />
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-[var(--radix-popover-trigger-width)] min-w-64 p-0"
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <Command shouldFilter={false}>
          <div className="border-b border-border p-2">
            <div className="flex h-8 items-center gap-2 rounded-md border border-input bg-field px-2">
              <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <input
                id={`${id}-search`}
                type="text"
                value={value}
                onChange={(event) => onChange(event.target.value)}
                placeholder={placeholder}
                autoComplete="off"
                data-1p-ignore="true"
                data-lpignore="true"
                data-bwignore="true"
                data-form-type="other"
                data-protonpass-ignore="true"
                name="media-server-user-search"
                className="w-full bg-transparent text-sm text-foreground placeholder:text-muted-foreground outline-none"
              />
            </div>
          </div>
          <CommandList>
            {loading ? (
              <div className="flex items-center gap-2 px-3 py-3 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                <span>{loadingLabel}</span>
              </div>
            ) : null}
            {!loading && filteredGroups.length === 0 ? (
              <CommandEmpty>{emptyLabel}</CommandEmpty>
            ) : null}
            {filteredGroups.map((group) => (
              <CommandGroup
                key={group.connectionId}
                heading={mediaServerUserGroupLabel(group)}
              >
                {group.status !== "READY" ? (
                  <div
                    id={selectorId(
                      "settings-external-invite-media-server-user-group-status",
                      group.provider,
                      group.connectionId,
                    )}
                    className="px-2 py-2 text-xs text-muted-foreground"
                  >
                    {group.errorMessage ?? emptyLabel}
                  </div>
                ) : group.users.length === 0 ? (
                  <div className="px-2 py-2 text-xs text-muted-foreground">
                    {serverEmptyLabel}
                  </div>
                ) : (
                  group.users.map((user) => {
                    const label = user.displayName ?? user.username;
                    const selected =
                      selectedOption?.id === user.id &&
                      selectedOption.connectionId === group.connectionId &&
                      selectedOption.provider === group.provider;
                    const option: ExternalInviteMediaServerUserOption = {
                      ...user,
                      provider: group.provider,
                      connectionId: group.connectionId,
                      connectionName: group.connectionName,
                    };
                    const optionId = selectorId(
                      "settings-external-invite-provider-user-option",
                      group.provider,
                      group.connectionId,
                      user.username,
                    );
                    return (
                      <CommandItem
                        key={`${group.connectionId}:${user.id}`}
                        value={`${user.username} ${user.displayName ?? ""} ${user.id} ${group.connectionName} ${group.provider}`}
                        onSelect={() => {
                          onSelectOption(option);
                          setOpen(false);
                        }}
                        className="items-center gap-3"
                      >
                        <ProviderAvatar
                          avatarUrl={user.avatarUrl}
                          label={label}
                        />
                        <span id={optionId} className="min-w-0 flex-1">
                          <span className="block truncate font-medium">
                            {label}
                          </span>
                          <span className="block truncate text-xs text-muted-foreground">
                            {user.username}
                          </span>
                        </span>
                        {selected ? (
                          <Check className="h-4 w-4 text-primary" />
                        ) : null}
                      </CommandItem>
                    );
                  })
                )}
              </CommandGroup>
            ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

function inviteStatus(
  account: LinkedAccount,
  t: ReturnType<typeof useTranslate>,
) {
  if (account.status === "DISABLED") {
    return {
      label: t("settings.externalAccountInviteStatusDisabled"),
      className: "border-destructive/40 bg-destructive/10 text-destructive",
    };
  }

  if (account.lastLoginAt) {
    return {
      label: t("settings.externalAccountInviteStatusLoggedIn"),
      className:
        "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]",
    };
  }

  if (account.status === "PENDING_CLAIM") {
    return {
      label: t("settings.externalAccountInviteStatusPending"),
      className:
        "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
    };
  }

  return {
    label: t("settings.externalAccountInviteStatusActive"),
    className:
      "border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[var(--scry-muted3)]",
  };
}

export function ExternalAccountInvitesPanel({
  users,
  invites,
  mediaServerUserGroups,
  mediaServerUserSearchLoading,
  mediaServerUserLookupError,
  externalAuthSettings,
  loading,
  externalInviteDraft,
  externalInviteSubmitting,
  updateExternalInviteDraft,
  createExternalAccountInvite,
  showMediaServersLink = false,
}: ExternalAccountInvitesPanelProps) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const inviteUnavailable =
    users.length === 0 ||
    (!loading &&
      !mediaServerUserSearchLoading &&
      mediaServerUserGroups.length === 0 &&
      !mediaServerUserLookupError);
  const inviteCreateDisabled =
    externalInviteSubmitting ||
    !externalInviteDraft.userId ||
    !externalInviteDraft.connectionId ||
    externalInviteDraft.providerUserId.trim().length === 0;
  const usersById = new Map(users.map((user) => [user.id, user.username]));
  const sortedInvites = invites
    .filter((invite) => isVisibleExternalAccountProvider(invite.provider))
    .sort((left, right) => {
      const rightTime = new Date(right.createdAt).getTime();
      const leftTime = new Date(left.createdAt).getTime();
      return (
        (Number.isNaN(rightTime) ? 0 : rightTime) -
        (Number.isNaN(leftTime) ? 0 : leftTime)
      );
    });
  const selectedMediaServerUser =
    mediaServerUserGroups
      .find(
        (group) =>
          group.provider === externalInviteDraft.provider &&
          group.connectionId === externalInviteDraft.connectionId,
      )
      ?.users.find((user) => user.id === externalInviteDraft.providerUserId) ??
    null;
  const selectedMediaServerUserOption = selectedMediaServerUser
    ? {
        ...selectedMediaServerUser,
        provider: externalInviteDraft.provider,
        connectionId: externalInviteDraft.connectionId,
        connectionName:
          mediaServerUserGroups.find(
            (group) =>
              group.provider === externalInviteDraft.provider &&
              group.connectionId === externalInviteDraft.connectionId,
          )?.connectionName ?? externalInviteDraft.connectionId,
      }
    : null;

  return (
    <div className={EXTERNAL_INVITES_PANEL_CLASS}>
      <div className={EXTERNAL_INVITES_PANEL_HEADER_CLASS}>
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex items-center gap-2">
            <h3 className={EXTERNAL_INVITES_PANEL_TITLE_CLASS}>
              {t("settings.externalAccountInvites")}
            </h3>
            {loading ? (
              <Loader2 className="h-4 w-4 animate-spin text-[var(--scry-muted3)]" />
            ) : null}
          </div>
          {showMediaServersLink ? (
            <Button asChild variant="primary" className="w-fit shrink-0">
              <Link to="/settings/media-servers">
                {t("settings.openMediaServers")}
              </Link>
            </Button>
          ) : null}
        </div>
      </div>

      <div className="space-y-4 p-4 sm:p-5">
        {!inviteUnavailable ? (
        <form
          id="settings-external-account-invite-form"
          className="space-y-4"
          onSubmit={createExternalAccountInvite}
        >
          <div className="grid gap-3 md:grid-cols-[minmax(0,18rem)_minmax(0,1fr)_auto]">
            <div className="space-y-1.5">
              <Label htmlFor="settings-external-invite-user">
                {t("settings.user")}
              </Label>
              <Select
                value={externalInviteDraft.userId}
                onValueChange={(userId) =>
                  updateExternalInviteDraft({ userId })
                }
                disabled={externalInviteSubmitting}
              >
                <SelectTrigger
                  id="settings-external-invite-user"
                  className="w-full"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {users.map((user) => (
                    <SelectItem
                      id={selectorId(
                        "settings-external-invite-user-option",
                        user.username,
                      )}
                      key={user.id}
                      value={user.id}
                    >
                      {user.username}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5">
              <div className="flex items-center justify-between gap-2">
                <Label htmlFor="settings-external-invite-provider-identifier">
                  {t("settings.mediaServerUser")}
                </Label>
                {mediaServerUserSearchLoading ? (
                  <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    {t("label.loading")}
                  </span>
                ) : null}
              </div>
              <MediaServerUserCombobox
                id="settings-external-invite-provider-identifier"
                value={externalInviteDraft.providerUserIdentifier}
                groups={mediaServerUserGroups}
                selectedOption={selectedMediaServerUserOption}
                loading={mediaServerUserSearchLoading}
                disabled={externalInviteSubmitting}
                placeholder={t("settings.mediaServerUserSearchPlaceholder")}
                emptyLabel={t("settings.mediaServerUserPickerEmpty")}
                serverEmptyLabel={t(
                  "settings.mediaServerUserPickerServerEmpty",
                )}
                loadingLabel={t("label.loading")}
                onChange={(providerUserIdentifier) =>
                  updateExternalInviteDraft({
                    providerUserIdentifier,
                    connectionId: "",
                    providerUserId: "",
                  })
                }
                onSelectOption={(option) =>
                  updateExternalInviteDraft({
                    provider: option.provider,
                    connectionId: option.connectionId,
                    providerUserIdentifier: option.username,
                    providerUserId: option.id,
                  })
                }
              />
              {mediaServerUserLookupError ? (
                <p className="text-xs text-destructive">
                  {mediaServerUserLookupError}
                </p>
              ) : null}
            </div>
            <div className="space-y-1.5">
              <div aria-hidden="true" className="h-3.5" />
              <Button
                id="settings-external-account-invite-create"
                type="submit"
                className="min-w-40"
                disabled={inviteCreateDisabled}
              >
                {externalInviteSubmitting ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Plus className="h-4 w-4" />
                )}
                {externalInviteSubmitting
                  ? t("label.saving")
                  : t("settings.createInvite")}
              </Button>
            </div>
          </div>
        </form>
        ) : null}

      <div className="space-y-3">
        <h4 className="text-sm font-medium text-[var(--scry-ink2)]">
          {t("settings.previousExternalAccountInvites")}
        </h4>
        <div className={EXTERNAL_INVITES_TABLE_SHELL_CLASS}>
          <div className="overflow-x-auto">
            <Table
              id="settings-external-account-invites-table"
              className="min-w-[1040px] table-fixed"
            >
              <TableHeader>
                <TableRow className={EXTERNAL_INVITES_TABLE_HEADER_ROW_CLASS}>
                  <TableHead className={cn("w-40", EXTERNAL_INVITES_TABLE_HEADER_CELL_CLASS)}>
                    {t("settings.user")}
                  </TableHead>
                  <TableHead className={cn("w-32", EXTERNAL_INVITES_TABLE_HEADER_CELL_CLASS)}>
                    {t("settings.provider")}
                  </TableHead>
                  <TableHead className={cn("w-44", EXTERNAL_INVITES_TABLE_HEADER_CELL_CLASS)}>
                    {t("profile.linkedAccountConnection")}
                  </TableHead>
                  <TableHead className={cn("w-52", EXTERNAL_INVITES_TABLE_HEADER_CELL_CLASS)}>
                    {t("settings.externalAccountInviteProviderUser")}
                  </TableHead>
                  <TableHead className={cn("w-36", EXTERNAL_INVITES_TABLE_HEADER_CELL_CLASS)}>
                    {t("profile.linkedAccountStatus")}
                  </TableHead>
                  <TableHead className={cn("w-36", EXTERNAL_INVITES_TABLE_HEADER_CELL_CLASS)}>
                    {t("settings.externalAccountInviteCreatedAt")}
                  </TableHead>
                  <TableHead className={cn("w-36", EXTERNAL_INVITES_TABLE_HEADER_CELL_CLASS)}>
                    {t("settings.externalAccountInviteLastLogin")}
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {loading ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-muted-foreground">
                      {t("label.loading")}
                    </TableCell>
                  </TableRow>
                ) : sortedInvites.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-muted-foreground">
                      {t("settings.noExternalAccountInvites")}
                    </TableCell>
                  </TableRow>
                ) : (
                  sortedInvites.map((invite) => {
                    const status = inviteStatus(invite, t);
                    return (
                      <TableRow
                        data-ui="settings-table-row"
                        key={invite.id}
                        id={selectorId(
                          "settings-external-account-invite-row",
                          usersById.get(invite.userId) ?? invite.userId,
                          invite.provider,
                          invite.username,
                        )}
                      >
                        <TableCell>
                          {usersById.get(invite.userId) ?? invite.userId}
                        </TableCell>
                        <TableCell>{providerLabel(invite.provider)}</TableCell>
                        <TableCell>
                          {inviteConnectionLabel(externalAuthSettings, invite)}
                        </TableCell>
                        <TableCell>
                          <div className="flex items-center gap-2">
                            <ProviderAvatar
                              avatarUrl={invite.avatarUrl}
                              label={invite.displayName ?? invite.username}
                            />
                            <span>{providerIdentityLabel(invite)}</span>
                          </div>
                        </TableCell>
                        <TableCell>
                          <span
                            className={`inline-flex rounded-full border px-2 py-0.5 text-xs font-medium ${status.className}`}
                          >
                            {status.label}
                          </span>
                        </TableCell>
                        <TableCell>
                          {formatTimestamp(invite.createdAt, dateTimeFormat)}
                        </TableCell>
                        <TableCell>
                          {formatTimestamp(invite.lastLoginAt, dateTimeFormat)}
                        </TableCell>
                      </TableRow>
                    );
                  })
                )}
              </TableBody>
            </Table>
          </div>
        </div>
      </div>
    </div>
    </div>
  );
}

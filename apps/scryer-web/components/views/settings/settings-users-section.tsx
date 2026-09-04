import * as React from "react";
import { Check, ChevronRight, KeyRound, Loader2, Plus, Power, PowerOff, ShieldOff, Trash2, X } from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { InfoHelp } from "@/components/common/info-help";
import {
  PermissionDropdowns,
  type LibraryPermissionDrafts,
} from "@/components/common/permission-checkboxes";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import type { LibraryRecord, UserRecord } from "@/lib/types";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";

const USERS_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const USERS_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const USERS_PANEL_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const USERS_INSET_CLASS =
  "rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-3";
const USERS_TABLE_SHELL_CLASS =
  "overflow-hidden rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]";
const USERS_TABLE_HEADER_ROW_CLASS =
  "border-[var(--scry-border3)] bg-[var(--scry-inset)] hover:bg-[var(--scry-inset)]";
const USERS_TABLE_HEADER_CELL_CLASS =
  "font-semibold text-[var(--scry-muted2)]";
type SettingsUsersSectionProps = {
  settingsUsers: UserRecord[];
  libraries: LibraryRecord[];
  externalAccountInvitesPanel: React.ReactNode;
  currentUserId?: string | null;
  appPermissions: string[];
  libraryPermissions: string[];
  newUsername: string;
  setNewUsername: (value: string) => void;
  newPassword: string;
  setNewPassword: (value: string) => void;
  loadPasswordMinLength: () => Promise<number | null>;
  newAppPermissions: string[];
  newLibraryPermissionDrafts: LibraryPermissionDrafts;
  canManagePermissions: boolean;
  setNewAppPermissions: (permissions: string[]) => void;
  updateNewLibraryPermissions: (changes: LibraryPermissionDrafts) => void;
  createUser: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  userPasswordDrafts: Record<string, string>;
  userAppPermissionDrafts: Record<string, string[]>;
  userLibraryPermissionDrafts: Record<string, LibraryPermissionDrafts>;
  updateUserPasswordDraft: (userId: string, value: string) => void;
  updateUserAppPermissionDraft: (userId: string, permissions: string[]) => void;
  updateUserLibraryPermissionDrafts: (
    userId: string,
    changes: LibraryPermissionDrafts,
  ) => void;
  mutatingUserId: string | null;
  setUserPassword: (
    userId: string,
    passwordMinLength: number | null,
  ) => Promise<boolean> | boolean;
  setUserAppPermissions: (userId: string, permissions?: string[]) => Promise<void> | void;
  setUserLibraryPermissions: (
    userId: string,
    changes?: LibraryPermissionDrafts,
  ) => Promise<void> | void;
  setUserLoginEnabled: (user: UserRecord) => Promise<void> | void;
  deleteUser: (user: UserRecord) => Promise<void> | void;
  resetUserMfa: (user: UserRecord) => Promise<void> | void;
};

function CollapsiblePermissionSection({
  id,
  title,
  children,
}: {
  id?: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <details
      id={id}
      className="group rounded-[10px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-2.5"
    >
      <summary className="flex cursor-pointer list-none items-center gap-2 text-sm font-medium text-[var(--scry-ink2)] [&::-webkit-details-marker]:hidden">
        <ChevronRight className="h-4 w-4 text-[var(--scry-muted3)] transition-transform group-open:rotate-90" />
        <span>{title}</span>
      </summary>
      <div className="mt-2">{children}</div>
    </details>
  );
}

function AuthFactorStatusBadge({ enabled }: { enabled: boolean }) {
  const t = useTranslate();
  return (
    <span
      className={cn(
        "inline-flex min-w-24 items-center justify-center rounded-full border px-2 py-1 text-xs font-medium",
        enabled
          ? "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]"
          : "border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[var(--scry-muted3)]",
      )}
    >
      {enabled ? t("settings.setUp") : t("settings.notSetUp")}
    </span>
  );
}

function UserLoginStatusBadge({ enabled }: { enabled: boolean }) {
  const t = useTranslate();
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium",
        enabled
          ? "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]"
          : "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
      )}
    >
      {t(enabled ? "settings.loginEnabled" : "settings.loginDisabled")}
    </span>
  );
}

export function SettingsUsersSection({
  settingsUsers,
  libraries,
  currentUserId,
  appPermissions,
  libraryPermissions,
  newUsername,
  setNewUsername,
  newPassword,
  setNewPassword,
  loadPasswordMinLength,
  newAppPermissions,
  newLibraryPermissionDrafts,
  canManagePermissions,
  setNewAppPermissions,
  updateNewLibraryPermissions,
  createUser,
  userPasswordDrafts,
  userAppPermissionDrafts,
  userLibraryPermissionDrafts,
  updateUserPasswordDraft,
  updateUserAppPermissionDraft,
  updateUserLibraryPermissionDrafts,
  mutatingUserId,
  setUserPassword,
  setUserAppPermissions,
  setUserLibraryPermissions,
  setUserLoginEnabled,
  deleteUser,
  resetUserMfa,
  externalAccountInvitesPanel,
}: SettingsUsersSectionProps) {
  const t = useTranslate();
  const [isCreateUserOpen, setIsCreateUserOpen] = React.useState(false);
  const [passwordResetUser, setPasswordResetUser] =
    React.useState<UserRecord | null>(null);
  const [passwordResetEditorUserId, setPasswordResetEditorUserId] =
    React.useState<string | null>(null);
  const [passwordMinLength, setPasswordMinLength] = React.useState<number | null>(null);
  const isPasswordTooShort = (password: string) =>
    passwordMinLength !== null &&
    password.length > 0 &&
    password.length < passwordMinLength;
  return (
    <div id="settings-users-section" className="space-y-4 text-sm">
      <div className={USERS_PANEL_CLASS}>
        <div className={USERS_PANEL_HEADER_CLASS}>
          <h3 className={USERS_PANEL_TITLE_CLASS}>
            {t("settings.knownUsers")}
          </h3>
        </div>
        <div className="p-4 sm:p-5">
          <div className={USERS_TABLE_SHELL_CLASS}>
            <div className="overflow-x-auto">
              <Table id="settings-users-table" className="min-w-[1180px] table-fixed">
                <TableHeader>
                  <TableRow className={USERS_TABLE_HEADER_ROW_CLASS}>
                    <TableHead className={cn("w-44", USERS_TABLE_HEADER_CELL_CLASS)}>
                      {t("settings.username")}
                    </TableHead>
                    <TableHead className={cn("w-[32rem]", USERS_TABLE_HEADER_CELL_CLASS)}>
                      {t("settings.permissions")}
                    </TableHead>
                    <TableHead className={cn("w-28", USERS_TABLE_HEADER_CELL_CLASS)}>
                      {t("settings.mfa")}
                    </TableHead>
                    <TableHead className={cn("w-28", USERS_TABLE_HEADER_CELL_CLASS)}>
                      {t("settings.passkey")}
                    </TableHead>
                    <TableHead className={cn("w-72", USERS_TABLE_HEADER_CELL_CLASS)}>
                      {t("settings.temporaryPassword")}
                    </TableHead>
                    <TableHead className={cn("w-44 text-right", USERS_TABLE_HEADER_CELL_CLASS)}>
                      {t("label.actions")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {settingsUsers.length === 0 ? (
                    <TableRow>
                      <TableCell
                        colSpan={6}
                        className="py-5 text-[var(--scry-muted3)]"
                      >
                        {t("settings.noUsers")}
                      </TableCell>
                    </TableRow>
                  ) : (
                    settingsUsers.map((user) => {
                      const isOwnUser = currentUserId === user.id;
                      const isRecoveryManaged =
                        user.username.toLowerCase() === "recovery-admin";
                      const canSetPassword =
                        user.accountKind !== "EXTERNAL_AUTO_PROVISIONED";
                      const permissionControlsDisabled =
                        mutatingUserId === user.id || !canManagePermissions;
                      const appSelected =
                        userAppPermissionDrafts[user.id] ?? user.appPermissions;
                      const libraryDrafts =
                        userLibraryPermissionDrafts[user.id] ??
                        Object.fromEntries(
                          user.libraryPermissions.map((grant) => [
                            grant.libraryId,
                            grant.permissions,
                          ]),
                        );
                      return (
                        <TableRow
                          key={user.id}
                          id={selectorId("settings-user-row", user.username)}
                          data-ui="settings-user-table-row"
                        >
                    <TableCell className="align-middle">
                      <div className="text-lg font-semibold text-[var(--scry-ink2)]">
                        {user.username}
                      </div>
                      <div className="mt-1">
                        <UserLoginStatusBadge enabled={user.loginEnabled} />
                      </div>
                      {user.passwordChangeRequired ? (
                        <div className="mt-1 text-xs text-[var(--scry-warning-text)]">
                          {t("settings.passwordChangeRequired")}
                        </div>
                      ) : null}
                    </TableCell>
                    <TableCell className="align-middle">
                      <CollapsiblePermissionSection
                        id={selectorId("settings-user-permissions", user.username, "section")}
                        title="Permissions"
                      >
                        <PermissionDropdowns
                          libraries={libraries}
                          appPermissions={appPermissions}
                          libraryPermissions={libraryPermissions}
                          idPrefix={selectorId("settings-user-permissions", user.username)}
                          selectedAppPermissions={appSelected}
                          selectedLibraryPermissions={libraryDrafts}
                          disabled={permissionControlsDisabled}
                          showSelectAll
                          onAppChange={(nextPermissions) => {
                            if (!canManagePermissions) return;
                            updateUserAppPermissionDraft(user.id, nextPermissions);
                            void setUserAppPermissions(user.id, nextPermissions);
                          }}
                          onLibraryChange={(changes) => {
                            if (!canManagePermissions) return;
                            updateUserLibraryPermissionDrafts(user.id, changes);
                            void setUserLibraryPermissions(user.id, changes);
                          }}
                        />
                      </CollapsiblePermissionSection>
                    </TableCell>
                    <TableCell className="align-middle">
                      <AuthFactorStatusBadge enabled={user.hasMfa} />
                    </TableCell>
                    <TableCell className="align-middle">
                      <AuthFactorStatusBadge enabled={user.hasPasskey} />
                    </TableCell>
                    <TableCell className="align-middle">
                      {canSetPassword && !isOwnUser ? (
                        passwordResetEditorUserId === user.id ? (
                          <div className="flex items-center gap-2">
                            <label className="sr-only" htmlFor={`new-password-${user.id}`}>
                              {t("settings.temporaryPassword")}
                            </label>
                            <div className="min-w-0 flex-1">
                              <Input
                                id={`new-password-${user.id}`}
                                value={userPasswordDrafts[user.id] ?? ""}
                                onChange={(event) =>
                                  updateUserPasswordDraft(user.id, event.target.value)
                                }
                                placeholder={t("form.newPasswordPlaceholder")}
                                type="password"
                                autoComplete="new-password"
                                minLength={passwordMinLength ?? undefined}
                                aria-label={t("settings.temporaryPassword")}
                                aria-invalid={isPasswordTooShort(
                                  userPasswordDrafts[user.id] ?? "",
                                )}
                                aria-describedby={
                                  isPasswordTooShort(userPasswordDrafts[user.id] ?? "")
                                    ? `new-password-${user.id}-error`
                                    : undefined
                                }
                                disabled={mutatingUserId === user.id}
                              />
                              {isPasswordTooShort(userPasswordDrafts[user.id] ?? "") ? (
                                <p
                                  id={`new-password-${user.id}-error`}
                                  className="mt-1 text-xs text-[var(--scry-danger-text)]"
                                >
                                  {t("settings.passwordMinLengthError", {
                                    min: passwordMinLength,
                                  })}
                                </p>
                              ) : null}
                            </div>
                            <IconButton
                              id={selectorId("settings-user-update-password", user.username)}
                              label={t("label.save")}
                              tone="enabled"
                              onClick={() => setPasswordResetUser(user)}
                              disabled={
                                mutatingUserId === user.id ||
                                !(userPasswordDrafts[user.id]?.trim()) ||
                                isPasswordTooShort(userPasswordDrafts[user.id] ?? "")
                              }
                            >
                              {mutatingUserId === user.id ? (
                                <Loader2 className="h-4 w-4 animate-spin" />
                              ) : (
                                <Check className="h-4 w-4" />
                              )}
                            </IconButton>
                            <IconButton
                              id={selectorId(
                                "settings-user-cancel-password-reset",
                                user.username,
                              )}
                              label={t("label.cancel")}
                              tone="delete"
                              onClick={() => {
                                updateUserPasswordDraft(user.id, "");
                                setPasswordResetEditorUserId(null);
                              }}
                              disabled={mutatingUserId === user.id}
                            >
                              <X className="h-4 w-4" />
                            </IconButton>
                          </div>
                        ) : (
                          <Button
                            id={selectorId("settings-user-reset-password", user.username)}
                            variant="primary"
                            size="sm"
                            onClick={async () => {
                              setPasswordMinLength(await loadPasswordMinLength());
                              setPasswordResetEditorUserId(user.id);
                            }}
                            disabled={mutatingUserId === user.id}
                          >
                            <KeyRound className="h-3.5 w-3.5" />
                            {t("settings.resetPassword")}
                          </Button>
                        )
                      ) : (
                        <span className="text-sm text-[var(--scry-muted3)]">
                          N/A
                        </span>
                      )}
                    </TableCell>
                    <TableCell className="align-middle text-right">
                      <div className="flex justify-end gap-2">
                        {!isOwnUser && (user.hasMfa || user.hasPasskey) ? (
                          <IconButton
                            id={selectorId("settings-user-reset-mfa", user.username)}
                            label={t("settings.resetMfa")}
                            tone={user.loginEnabled ? "disabled" : "enabled"}
                            onClick={() => void resetUserMfa(user)}
                            disabled={mutatingUserId === user.id}
                          >
                            <ShieldOff className="h-4 w-4" />
                          </IconButton>
                        ) : null}
                        {!isRecoveryManaged ? (
                          <IconButton
                            id={selectorId("settings-user-login-status", user.username)}
                            label={t(
                              user.loginEnabled
                                ? "settings.disableLogin"
                                : "settings.enableLogin",
                            )}
                            tone={user.loginEnabled ? "disabled" : "enabled"}
                            onClick={() => void setUserLoginEnabled(user)}
                            disabled={mutatingUserId === user.id || isOwnUser}
                          >
                            {user.loginEnabled ? (
                              <PowerOff className="h-4 w-4" />
                            ) : (
                              <Power className="h-4 w-4" />
                            )}
                          </IconButton>
                        ) : null}
                        {!user.isDefaultAdmin ? (
                          <IconButton
                            id={selectorId("settings-user-delete", user.username)}
                            label={t("label.delete")}
                            tone="delete"
                            onClick={() => void deleteUser(user)}
                            disabled={mutatingUserId === user.id || isOwnUser}
                          >
                            <Trash2 className="h-4 w-4" />
                          </IconButton>
                        ) : null}
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
        </div>
      </div>

      {isCreateUserOpen ? (
        <div className={USERS_PANEL_CLASS}>
          <div className={USERS_PANEL_HEADER_CLASS}>
            <h3 className={USERS_PANEL_TITLE_CLASS}>
              {t("settings.createUser")}
            </h3>
          </div>
          <div className="p-4 sm:p-5">
            <form id="settings-user-create-form" className="space-y-4" onSubmit={createUser}>
              <div className="grid gap-3 md:grid-cols-2">
                <div>
                  <Label
                    htmlFor="settings-user-username"
                    className="mb-2 flex min-h-5 items-center"
                  >
                    {t("settings.username")}
                  </Label>
                  <Input
                    id="settings-user-username"
                    value={newUsername}
                    onChange={(event) => setNewUsername(event.target.value)}
                    placeholder={t("form.usernamePlaceholder")}
                    autoComplete="off"
                    required
                  />
                </div>
                <div>
                  <Label
                    htmlFor="settings-user-password"
                    className="mb-2 flex min-h-5 items-center gap-1.5"
                  >
                    {t("settings.temporaryPassword")}
                    <InfoHelp
                      ariaLabel={t("settings.temporaryPassword")}
                      text={t("settings.temporaryPasswordHelp")}
                    />
                  </Label>
                  <Input
                    id="settings-user-password"
                    value={newPassword}
                    onChange={(event) => setNewPassword(event.target.value)}
                    placeholder={t("form.passwordPlaceholder")}
                    type="password"
                    autoComplete="new-password"
                    required
                  />
                </div>
              </div>
              {canManagePermissions ? (
                <div className={USERS_INSET_CLASS}>
                  <Label className="mb-2 block text-[var(--scry-ink2)]">
                    {t("settings.permissions")}
                  </Label>
                  <PermissionDropdowns
                    libraries={libraries}
                    appPermissions={appPermissions}
                    libraryPermissions={libraryPermissions}
                    selectedAppPermissions={newAppPermissions}
                    selectedLibraryPermissions={newLibraryPermissionDrafts}
                    showSelectAll
                    onAppChange={setNewAppPermissions}
                    onLibraryChange={updateNewLibraryPermissions}
                  />
                </div>
              ) : null}
              <div className="flex flex-wrap gap-2">
                <Button
                  id="settings-user-create"
                  type="submit"
                >
                  <Plus className="h-4 w-4" />
                  {t("settings.createUser")}
                </Button>
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => setIsCreateUserOpen(false)}
                >
                  {t("label.cancel")}
                </Button>
              </div>
            </form>
          </div>
        </div>
      ) : (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-user-create-open"
            icon={Plus}
            label={t("settings.createUser")}
            onClick={() => setIsCreateUserOpen(true)}
          />
        </div>
      )}

      <ConfirmDialog
        open={passwordResetUser !== null}
        contentId="settings-user-temporary-password-dialog"
        title={t("settings.temporaryPasswordResetTitle")}
        description={t("settings.temporaryPasswordResetDescription", {
          name: passwordResetUser?.username ?? "",
        })}
        confirmLabel={t("settings.setTemporaryPassword")}
        cancelLabel={t("label.cancel")}
        confirmButtonVariant="destructive"
        isBusy={
          passwordResetUser !== null && mutatingUserId === passwordResetUser.id
        }
        onConfirm={async () => {
          if (!passwordResetUser) return;
          const saved = await setUserPassword(passwordResetUser.id, passwordMinLength);
          if (saved) {
            setPasswordResetEditorUserId(null);
            setPasswordResetUser(null);
          }
        }}
        onCancel={() => setPasswordResetUser(null)}
      />

      {externalAccountInvitesPanel}
    </div>
  );
}


import type * as React from "react";
import { KeyRound, Trash2, User2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
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
import type { Translate } from "@/components/root/types";

type UserRecord = {
  id: string;
  username: string;
  entitlements: string[];
};

type SettingsUsersSectionProps = {
  t: Translate;
  settingsUsers: UserRecord[];
  currentUserId?: string | null;
  ALL_ENTITLEMENTS: string[];
  humanizeEntitlement: (entitlement: string) => string;
  newUsername: string;
  setNewUsername: (value: string) => void;
  newPassword: string;
  setNewPassword: (value: string) => void;
  newEntitlements: string[];
  toggleNewEntitlement: (value: string) => void;
  createUser: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  userPasswordDrafts: Record<string, string>;
  userEntitlementDrafts: Record<string, string[]>;
  updateUserPasswordDraft: (userId: string, value: string) => void;
  toggleUserEntitlement: (userId: string, entitlement: string) => void;
  mutatingUserId: string | null;
  setUserPassword: (userId: string) => Promise<void> | void;
  setUserEntitlements: (userId: string, entitlements?: string[]) => Promise<void> | void;
  deleteUser: (user: UserRecord) => Promise<void> | void;
};

export function SettingsUsersSection({
  t,
  settingsUsers,
  currentUserId,
  ALL_ENTITLEMENTS,
  humanizeEntitlement,
  newUsername,
  setNewUsername,
  newPassword,
  setNewPassword,
  newEntitlements,
  toggleNewEntitlement,
  createUser,
  userPasswordDrafts,
  userEntitlementDrafts,
  updateUserPasswordDraft,
  toggleUserEntitlement,
  mutatingUserId,
  setUserPassword,
  setUserEntitlements,
  deleteUser,
}: SettingsUsersSectionProps) {
  return (
    <div className="space-y-4 text-sm">
      <CardTitle className="flex items-center gap-2 text-base">
        <User2 className="h-4 w-4" />
        {t("settings.knownUsers")}
      </CardTitle>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">{t("settings.createUser")}</CardTitle>
        </CardHeader>
        <CardContent>
          <form className="space-y-3" onSubmit={createUser}>
            <div className="grid gap-3 md:grid-cols-3">
              <div>
                <Label htmlFor="settings-user-username" className="mb-2 block">
                  {t("settings.username")}
                </Label>
                <Input
                  id="settings-user-username"
                  value={newUsername}
                  onChange={(event) => setNewUsername(event.target.value)}
                  placeholder={t("form.usernamePlaceholder")}
                  required
                />
              </div>
              <div>
                <Label htmlFor="settings-user-password" className="mb-2 block">
                  {t("settings.password")}
                </Label>
                <Input
                  id="settings-user-password"
                  value={newPassword}
                  onChange={(event) => setNewPassword(event.target.value)}
                  placeholder={t("form.passwordPlaceholder")}
                  type="password"
                  required
                />
              </div>
              <div>
                <Label className="mb-2 block">{t("settings.entitlements")}</Label>
                <div className="grid rounded border border-border bg-background/40 p-2">
                  {ALL_ENTITLEMENTS.map((entitlement) => (
                    <label
                      key={`new-${entitlement}`}
                      className="flex items-center gap-3 rounded-md px-2 py-1.5 hover:bg-card/70"
                    >
                      <Checkbox
                        checked={newEntitlements.includes(entitlement)}
                        onCheckedChange={() => toggleNewEntitlement(entitlement)}
                      />
                      <span>{humanizeEntitlement(entitlement)}</span>
                    </label>
                  ))}
                </div>
              </div>
            </div>
            <Button type="submit" className="min-w-40">
              {t("settings.createUser")}
            </Button>
          </form>
        </CardContent>
      </Card>

      <div className="rounded border border-border">
        <div className="border-b border-border px-3 py-2">
          <CardTitle className="text-base">{t("settings.knownUsers")}</CardTitle>
        </div>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="min-w-40">{t("settings.username")}</TableHead>
              <TableHead className="min-w-[360px]">{t("settings.entitlements")}</TableHead>
              <TableHead className="min-w-72">{t("settings.newPassword")}</TableHead>
              <TableHead className="w-44 text-right">{t("settings.actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
                {settingsUsers.length === 0 ? (
              <TableRow>
                <TableCell colSpan={4} className="text-muted-foreground">
                  {t("settings.noUsers")}
                </TableCell>
              </TableRow>
            ) : (
              settingsUsers.map((user) => {
                const isOwnUser = currentUserId === user.id;
                return (
                  <TableRow key={user.id}>
                    <TableCell className="align-middle">
                      <div className="text-lg font-semibold text-foreground">{user.username}</div>
                    </TableCell>
                    <TableCell className="align-top">
                      <div className="space-y-2">
                        <div className="grid max-h-32 gap-1 overflow-y-auto rounded border border-border bg-background/40 p-2 md:grid-cols-2">
                          {ALL_ENTITLEMENTS.map((entitlement) => (
                            <label
                              key={`user-${user.id}-${entitlement}`}
                              className="flex items-center gap-2"
                            >
                              <Checkbox
                                checked={(userEntitlementDrafts[user.id] ?? user.entitlements).includes(entitlement)}
                                onCheckedChange={() => {
                                  if (isOwnUser) {
                                    return;
                                  }
                                  const currentEntitlements = userEntitlementDrafts[user.id] ?? user.entitlements;
                                  const nextSet = new Set(currentEntitlements);
                                  if (nextSet.has(entitlement)) {
                                    nextSet.delete(entitlement);
                                  } else {
                                    nextSet.add(entitlement);
                                  }
                                  const nextEntitlements = Array.from(nextSet);
                                  toggleUserEntitlement(user.id, entitlement);
                                  void setUserEntitlements(user.id, nextEntitlements);
                                }}
                                disabled={mutatingUserId === user.id || isOwnUser}
                              />
                              <span>{humanizeEntitlement(entitlement)}</span>
                            </label>
                          ))}
                        </div>
                      </div>
                    </TableCell>
                    <TableCell className="align-middle">
                      <div className="flex items-center gap-2">
                        <label className="sr-only" htmlFor={`new-password-${user.id}`}>
                          {t("settings.newPassword")}
                        </label>
                        <Input
                          id={`new-password-${user.id}`}
                          value={userPasswordDrafts[user.id] ?? ""}
                          onChange={(event) => updateUserPasswordDraft(user.id, event.target.value)}
                          placeholder={t("form.newPasswordPlaceholder")}
                          type="password"
                          aria-label={t("settings.newPassword")}
                        />
                        <Button
                          variant="secondary"
                          size="sm"
                          className="min-w-44"
                          onClick={() => void setUserPassword(user.id)}
                          disabled={mutatingUserId === user.id}
                        >
                          <KeyRound className="mr-1 h-3.5 w-3.5" />
                          {mutatingUserId === user.id ? t("label.saving") : t("settings.updatePassword")}
                        </Button>
                      </div>
                    </TableCell>
                    <TableCell className="align-middle text-right">
                      <div className="flex justify-end gap-2">
                        <Button
                          variant="destructive"
                          size="sm"
                          onClick={() => void deleteUser(user)}
                          disabled={mutatingUserId === user.id || isOwnUser}
                        >
                          <Trash2 className="mr-1 h-3.5 w-3.5" />
                          {t("label.delete")}
                        </Button>
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
  );
}

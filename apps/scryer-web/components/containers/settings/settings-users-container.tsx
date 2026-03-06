
import { useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsUsersSection } from "@/components/views/settings/settings-users-section";
import { ALL_ENTITLEMENTS } from "@/lib/constants/entitlements";
import {
  createUserMutation,
  deleteUserMutation,
  setUserEntitlementsMutation,
  setUserPasswordMutation,
} from "@/lib/graphql/mutations";
import { usersQuery } from "@/lib/graphql/queries";
import { useAuth } from "@/lib/hooks/use-auth";
import { humanizeEntitlement } from "@/lib/utils/formatting";
import { useClient } from "urql";
import type { UserRecord } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";

function normalizeEntitlementValue(value: string): string {
  const normalized = value.trim().toLowerCase().replace(/[-\s]/g, "_");
  const compact = normalized.replace(/_/g, "");
  switch (compact) {
    case "viewcatalog":
      return "view_catalog";
    case "monitortitle":
      return "monitor_title";
    case "managetitle":
      return "manage_title";
    case "triggeractions":
      return "trigger_actions";
    case "manageconfig":
      return "manage_config";
    case "viewhistory":
      return "view_history";
    default:
      return normalized;
  }
}

function normalizeEntitlements(values: string[]): string[] {
  return Array.from(new Set(values.map(normalizeEntitlementValue).filter((value) => value.length > 0)));
}

export function SettingsUsersContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { user: currentUser } = useAuth();
  const [settingsUsers, setSettingsUsers] = useState<UserRecord[]>([]);
  const [newUsername, setNewUsername] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [newEntitlements, setNewEntitlements] = useState<string[]>([]);
  const [userPasswordDrafts, setUserPasswordDrafts] = useState<Record<string, string>>({});
  const [userEntitlementDrafts, setUserEntitlementDrafts] = useState<Record<string, string[]>>({});
  const [mutatingUserId, setMutatingUserId] = useState<string | null>(null);
  const [pendingDeleteUser, setPendingDeleteUser] = useState<UserRecord | null>(null);

  const toggleEntitlement = useCallback((current: string[], value: string) => {
    const existing = new Set(current);
    if (existing.has(value)) {
      existing.delete(value);
    } else {
      existing.add(value);
    }
    return Array.from(existing);
  }, []);

  const updateUserPasswordDraft = useCallback((userId: string, value: string) => {
    setUserPasswordDrafts((previous) => ({ ...previous, [userId]: value }));
  }, []);

  const toggleNewEntitlement = useCallback((value: string) => {
    setNewEntitlements((previous) => toggleEntitlement(previous, value));
  }, [toggleEntitlement]);

  const toggleUserEntitlement = useCallback((userId: string, value: string) => {
    setUserEntitlementDrafts((previous) => ({
      ...previous,
      [userId]: toggleEntitlement(previous[userId] ?? [], value),
    }));
  }, [toggleEntitlement]);

  const refreshUsers = useCallback(async () => {
    try {
      const { data, error } = await client.query(usersQuery, {}).toPromise();
      if (error) throw error;
      const users = (data.users || []).map((user: { id: string; username: string; entitlements: string[] }) => ({
        ...user,
        entitlements: normalizeEntitlements(user.entitlements ?? []),
      }));
      setSettingsUsers(users);
      setUserEntitlementDrafts(
        Object.fromEntries(users.map((user: { id: string; username: string; entitlements: string[] }) => [user.id, [...user.entitlements]])),
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshUsers();
  }, [refreshUsers]);

  const createUser = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!newUsername.trim() || !newPassword.trim()) {
      setGlobalStatus(t("status.userRequired"));
      return;
    }
    try {
      const { error } = await client.mutation(createUserMutation, {
        input: {
          username: newUsername.trim(),
          password: newPassword,
          entitlements: newEntitlements,
        },
      }).toPromise();
      if (error) throw error;
      setNewUsername("");
      setNewPassword("");
      setNewEntitlements([]);
      setGlobalStatus(t("user.created"));
      await refreshUsers();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToCreate"));
    }
  };

  const setUserPassword = async (userId: string) => {
    const password = userPasswordDrafts[userId]?.trim();
    if (!password) {
      setGlobalStatus(t("status.passwordRequired"));
      return;
    }
    setMutatingUserId(userId);
    try {
      const { error } = await client.mutation(setUserPasswordMutation, {
        input: {
          userId,
          password,
        },
      }).toPromise();
      if (error) throw error;
      setUserPasswordDrafts((previous) => ({
        ...previous,
        [userId]: "",
      }));
      setGlobalStatus(t("user.passwordUpdated"));
      await refreshUsers();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingUserId(null);
    }
  };

  const setUserEntitlements = async (userId: string, entitlements?: string[]) => {
    const resolvedEntitlements = entitlements ?? userEntitlementDrafts[userId] ?? [];
    setMutatingUserId(userId);
    try {
      const { error } = await client.mutation(setUserEntitlementsMutation, {
        input: {
          userId,
          entitlements: resolvedEntitlements,
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("user.entitlementsUpdated"));
      await refreshUsers();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingUserId(null);
    }
  };

  const deleteUser = async (user: UserRecord) => {
    setPendingDeleteUser(user);
  };

  const confirmDeleteUser = async () => {
    if (!pendingDeleteUser) {
      return;
    }
    const user = pendingDeleteUser;
    setMutatingUserId(user.id);
    try {
      const { error } = await client.mutation(deleteUserMutation, {
        input: { userId: user.id },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.deletingUser", { name: user.username }));
      await refreshUsers();
      setUserPasswordDrafts((previous) => {
        const next = { ...previous };
        delete next[user.id];
        return next;
      });
      setUserEntitlementDrafts((previous) => {
        const next = { ...previous };
        delete next[user.id];
        return next;
      });
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingUserId(null);
      setPendingDeleteUser(null);
    }
  };

  return (
    <>
      <SettingsUsersSection
        settingsUsers={settingsUsers}
        newUsername={newUsername}
        setNewUsername={setNewUsername}
        newPassword={newPassword}
        setNewPassword={setNewPassword}
        newEntitlements={newEntitlements}
        toggleNewEntitlement={toggleNewEntitlement}
        createUser={createUser}
        userPasswordDrafts={userPasswordDrafts}
        userEntitlementDrafts={userEntitlementDrafts}
        updateUserPasswordDraft={updateUserPasswordDraft}
        toggleUserEntitlement={toggleUserEntitlement}
        mutatingUserId={mutatingUserId}
        setUserPassword={setUserPassword}
        setUserEntitlements={setUserEntitlements}
        deleteUser={deleteUser}
        currentUserId={currentUser?.id ?? null}
        ALL_ENTITLEMENTS={[...ALL_ENTITLEMENTS]}
        humanizeEntitlement={humanizeEntitlement}
      />
      <ConfirmDialog
        open={pendingDeleteUser !== null}
        title={t("label.delete")}
        description={pendingDeleteUser ? t("status.deletingUser", { name: pendingDeleteUser.username }) : ""}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingUserId !== null}
        onConfirm={confirmDeleteUser}
        onCancel={() => setPendingDeleteUser(null)}
      />
    </>
  );
}

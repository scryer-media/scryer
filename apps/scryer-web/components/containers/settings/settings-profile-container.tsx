import { useState, useCallback } from "react";
import { useClient } from "urql";
import { SettingsProfileSection } from "@/components/views/settings/settings-profile-section";
import { setUserPasswordMutation } from "@/lib/graphql/mutations";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";

type Props = {
  userId?: string;
  username?: string;
};

export function SettingsProfileContainer({ userId, username }: Props) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [saving, setSaving] = useState(false);

  const handleChangePassword = useCallback(async () => {
    if (!userId || !newPassword || newPassword !== confirmPassword) return;

    setSaving(true);
    try {
      const result = await client
        .mutation(setUserPasswordMutation, {
          input: {
            userId,
            password: newPassword,
            currentPassword,
          },
        })
        .toPromise();

      if (result.error) {
        setGlobalStatus(result.error.message);
        return;
      }

      setCurrentPassword("");
      setNewPassword("");
      setConfirmPassword("");
      setGlobalStatus(t("profile.passwordUpdated"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setSaving(false);
    }
  }, [client, userId, currentPassword, newPassword, confirmPassword, setGlobalStatus, t]);

  return (
    <SettingsProfileSection
      username={username}
      currentPassword={currentPassword}
      newPassword={newPassword}
      confirmPassword={confirmPassword}
      saving={saving}
      onCurrentPasswordChange={setCurrentPassword}
      onNewPasswordChange={setNewPassword}
      onConfirmPasswordChange={setConfirmPassword}
      onChangePassword={handleChangePassword}
    />
  );
}

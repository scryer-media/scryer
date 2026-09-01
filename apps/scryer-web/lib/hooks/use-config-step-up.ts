import { useCallback, useEffect, useRef, useState } from "react";

import { backendClient, MFA_STEP_UP_REQUIRED_EVENT } from "@/lib/graphql/urql-client";
import { authRuntimeStateQuery } from "@/lib/graphql/queries";
import { mfaVerifyStepUpMutation } from "@/lib/graphql/mutations";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";
import { scheduleAfterFirstPaint } from "@/lib/utils/scheduling";
import { decodeJwtPayload, jwtDateClaimToMillis } from "@/lib/utils/jwt";
import type { AuthUser } from "@/lib/hooks/use-auth";
import type { SetGlobalStatus } from "@/lib/context/global-status-context";
import type {
  ContentSettingsSection,
  SettingsSection,
  SystemSection,
  Translate,
  ViewId,
  WantedSection,
} from "@/components/root/types";

const CONFIG_STEP_UP_EXPIRY_LEEWAY_MS = 1_000;

function configStepUpExpiresAt(token: string | null): number | null {
  if (!token) {
    return null;
  }

  return jwtDateClaimToMillis(decodeJwtPayload(token)?.mfaStepUpVerifiedUntil);
}

function hasFreshConfigStepUp(token: string | null, now: number): boolean {
  const expiresAt = configStepUpExpiresAt(token);
  return (
    expiresAt !== null && expiresAt - CONFIG_STEP_UP_EXPIRY_LEEWAY_MS > now
  );
}

export function useConfigStepUp({
  authToken,
  initialMfaRequireConfigStepUp,
  protectedSettingsRoute,
  settingsSubscriptionEnabled,
  adoptSession,
  setGlobalStatus,
  navigateTo,
  t,
}: {
  authToken: string | null;
  initialMfaRequireConfigStepUp: boolean | null;
  protectedSettingsRoute: boolean;
  settingsSubscriptionEnabled: boolean;
  adoptSession: (nextToken: string, nextUser: AuthUser | null) => void;
  setGlobalStatus: SetGlobalStatus;
  navigateTo: (
    view: ViewId,
    settingsSection?: SettingsSection,
    contentSection?: ContentSettingsSection,
    systemSection?: SystemSection,
    wantedSection?: WantedSection,
  ) => void;
  t: Translate;
}) {
  const [configStepUpPolicy, setConfigStepUpPolicy] = useState({
    loading: initialMfaRequireConfigStepUp === null,
    required: initialMfaRequireConfigStepUp === true,
    error: false,
    resolved: initialMfaRequireConfigStepUp !== null,
  });
  const usedInitialPolicyRef = useRef(false);
  const [configStepUpNow, setConfigStepUpNow] = useState(() => Date.now());
  const [settingsStepUpCode, setSettingsStepUpCode] = useState("");
  const [settingsStepUpBusy, setSettingsStepUpBusy] = useState(false);
  const [settingsStepUpError, setSettingsStepUpError] = useState<string | null>(
    null,
  );
  const [settingsStepUpForced, setSettingsStepUpForced] = useState(false);

  const refreshConfigStepUpPolicy = useCallback(async (options?: { force?: boolean }) => {
    if (!protectedSettingsRoute) {
      usedInitialPolicyRef.current = false;
      setConfigStepUpPolicy((current) => ({
        ...current,
        loading: false,
        error: false,
      }));
      return;
    }

    if (
      options?.force !== true &&
      !usedInitialPolicyRef.current &&
      initialMfaRequireConfigStepUp !== null
    ) {
      usedInitialPolicyRef.current = true;
      setConfigStepUpPolicy({
        loading: false,
        required: initialMfaRequireConfigStepUp,
        error: false,
        resolved: true,
      });
      return;
    }

    usedInitialPolicyRef.current = true;
    setConfigStepUpPolicy((current) => ({
      ...current,
      loading: !current.resolved,
      error: false,
    }));
    try {
      const { data, error } = await backendClient
        .query<{
          authRuntimeState?: {
            mfaRequireConfigStepUp?: boolean | null;
          } | null;
        }>(authRuntimeStateQuery, {})
        .toPromise();
      if (error) {
        throw error;
      }

      const runtimeState = data?.authRuntimeState;
      setConfigStepUpPolicy({
        loading: false,
        required: runtimeState?.mfaRequireConfigStepUp === true,
        error: false,
        resolved: true,
      });
    } catch (error) {
      console.warn("Failed to refresh MFA step-up policy", error);
      setConfigStepUpPolicy((current) => ({
        ...current,
        loading: false,
        error: !current.resolved,
      }));
    }
  }, [initialMfaRequireConfigStepUp, protectedSettingsRoute]);

  useEffect(() => {
    return scheduleAfterFirstPaint(() => {
      void refreshConfigStepUpPolicy();
    });
  }, [refreshConfigStepUpPolicy]);

  useSettingsSubscription(
    useCallback(
      (changedKeys) => {
        if (
          changedKeys.includes("auth.mfa.require_config_step_up") ||
          changedKeys.includes("auth.totp.require_config_step_up") ||
          changedKeys.includes("auth.form_login_enabled") ||
          changedKeys.includes("auth.form.enabled")
        ) {
          void refreshConfigStepUpPolicy({ force: true });
        }
      },
      [refreshConfigStepUpPolicy],
    ),
    { enabled: settingsSubscriptionEnabled },
  );

  useEffect(() => {
    const expiresAt = configStepUpExpiresAt(authToken);
    if (expiresAt === null || typeof window === "undefined") {
      return;
    }

    const delay = Math.max(
      0,
      expiresAt - Date.now() - CONFIG_STEP_UP_EXPIRY_LEEWAY_MS + 250,
    );
    const timeoutId = window.setTimeout(
      () => setConfigStepUpNow(Date.now()),
      delay,
    );
    return () => window.clearTimeout(timeoutId);
  }, [authToken]);

  useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    const refreshClock = () => setConfigStepUpNow(Date.now());
    window.addEventListener("focus", refreshClock);
    return () => window.removeEventListener("focus", refreshClock);
  }, []);

  const configStepUpFresh = hasFreshConfigStepUp(authToken, configStepUpNow);
  const settingsStepUpOpen =
    protectedSettingsRoute &&
    !configStepUpPolicy.loading &&
    !configStepUpPolicy.error &&
    configStepUpPolicy.required &&
    (!configStepUpFresh || settingsStepUpForced);
  const settingsStepUpPolicyLoadFailed =
    protectedSettingsRoute && configStepUpPolicy.error;
  const settingsStepUpBlocksContent =
    protectedSettingsRoute &&
    (configStepUpPolicy.loading ||
      settingsStepUpPolicyLoadFailed ||
      settingsStepUpOpen);

  const navigateToSettingsProfile = useCallback(() => {
    navigateTo("settings", "profile", undefined, undefined, undefined);
  }, [navigateTo]);

  const handleCancelSettingsStepUp = useCallback(() => {
    setSettingsStepUpCode("");
    setSettingsStepUpError(null);
    setSettingsStepUpForced(false);
    navigateToSettingsProfile();
  }, [navigateToSettingsProfile]);

  const handleSettingsStepUpSubmit = useCallback(async () => {
    if (settingsStepUpCode.length !== 6) {
      return;
    }

    setSettingsStepUpBusy(true);
    setSettingsStepUpError(null);
    try {
      const result = await backendClient
        .mutation<
          { mfaVerifyStepUp?: { token: string; user: AuthUser | null } | null },
          { input: { code: string } }
        >(mfaVerifyStepUpMutation, { input: { code: settingsStepUpCode } })
        .toPromise();

      if (result.error || !result.data?.mfaVerifyStepUp) {
        const message = t("settings.mfaStepUpFailed");
        setSettingsStepUpCode("");
        setSettingsStepUpError(null);
        setSettingsStepUpForced(false);
        setGlobalStatus(message);
        navigateToSettingsProfile();
        return;
      }

      adoptSession(
        result.data.mfaVerifyStepUp.token,
        result.data.mfaVerifyStepUp.user,
      );
      setSettingsStepUpCode("");
      setSettingsStepUpError(null);
      setSettingsStepUpForced(false);
      setConfigStepUpNow(Date.now());
      setGlobalStatus(t("settings.mfaStepUpVerified"));
    } catch {
      const message = t("settings.mfaStepUpFailed");
      setSettingsStepUpCode("");
      setSettingsStepUpError(null);
      setSettingsStepUpForced(false);
      setGlobalStatus(message);
      navigateToSettingsProfile();
    } finally {
      setSettingsStepUpBusy(false);
    }
  }, [
    adoptSession,
    navigateToSettingsProfile,
    setGlobalStatus,
    settingsStepUpCode,
    t,
  ]);

  useEffect(() => {
    if (!configStepUpFresh) {
      return;
    }

    setSettingsStepUpForced(false);
  }, [configStepUpFresh]);

  useEffect(() => {
    if (settingsStepUpOpen) {
      return;
    }

    setSettingsStepUpCode("");
    setSettingsStepUpError(null);
  }, [settingsStepUpOpen]);

  useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    const handleMfaStepUpRequired = () => {
      if (!protectedSettingsRoute || !configStepUpPolicy.required) {
        return;
      }

      setSettingsStepUpForced(true);
      setSettingsStepUpError(t("settings.mfaStepUpRequiredAgain"));
      setGlobalStatus(t("settings.mfaStepUpRequiredAgain"));
    };

    window.addEventListener(
      MFA_STEP_UP_REQUIRED_EVENT,
      handleMfaStepUpRequired,
    );
    return () =>
      window.removeEventListener(
        MFA_STEP_UP_REQUIRED_EVENT,
        handleMfaStepUpRequired,
      );
  }, [configStepUpPolicy.required, protectedSettingsRoute, setGlobalStatus, t]);

  return {
    refreshConfigStepUpPolicy,
    settingsStepUpCode,
    setSettingsStepUpCode,
    settingsStepUpBusy,
    settingsStepUpError,
    settingsStepUpOpen,
    settingsStepUpPolicyLoadFailed,
    settingsStepUpBlocksContent,
    handleCancelSettingsStepUp,
    handleSettingsStepUpSubmit,
  };
}

import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { useTheme } from "next-themes";
import { Fingerprint, KeyRound, Loader2 } from "lucide-react";
import { TotpQrCode } from "@/components/common/totp-qr-code";
import { useAuth, type AuthUser } from "@/lib/hooks/use-auth";
import { useLanguage } from "@/lib/hooks/use-language";
import { TotpCodeForm, sanitizeTotpCode } from "@/components/auth/totp-code-form";
import { Input, integerInputProps } from "@/components/ui/input";
import { useBackendRestarting } from "@/lib/hooks/use-backend-restarting";
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";
import { isVisibleExternalAccountProvider } from "@/lib/constants/integration-providers";
import {
  backendClient,
  mfaEnrollmentClient,
  passwordChangeRequiredClient,
} from "@/lib/graphql/urql-client";
import { externalAuthRuntimeSettingsQuery } from "@/lib/graphql/queries";
import {
  completeRequiredPasswordChangeMutation,
  completeLoginMfaEnrollmentMutation,
  loginVerificationTotpCompleteMutation,
  loginWithEmbyMutation,
  loginWithJellyfinMutation,
  loginWithPlexMutation,
  totpEnrollmentStartMutation,
} from "@/lib/graphql/mutations";
import type {
  ExternalAuthRuntimeSettings,
  TotpEnrollmentComplete,
  TotpEnrollmentStart,
} from "@/lib/types/settings";
import {
  authenticateLoginVerificationPasskey,
  authenticateWithPasskey,
  PasskeyClientError,
  registerLoginEnrollmentPasskey,
} from "@/lib/utils/passkeys";
import { authenticateWithPlexPin } from "@/lib/utils/plex-oauth";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";

type LoginMethod = "password" | "jellyfin" | "emby" | null;

type LoginPayload = {
  token: string;
  user: AuthUser | null;
  mfaEnrollmentRequired: boolean;
  passwordChangeRequired?: boolean;
  mfaVerifiedUntil: string | null;
  persistSession: boolean;
};

type CompletedLoginPayload = Pick<
  LoginPayload,
  "token" | "user" | "passwordChangeRequired" | "persistSession"
>;

type LoginVerificationChallenge = {
  loginChallengeId: string;
  expiresAt: string;
  hasPasskey: boolean;
  hasTotp: boolean;
};

function resolveRedirectTarget(value: string | null): string {
  if (!value || !value.startsWith("/") || value.startsWith("//")) {
    return "/";
  }

  return value;
}

function connectionOptionLabel(connection: { displayName: string }): string {
  return connection.displayName;
}

function graphQlErrorCode(error: unknown): string | null {
  if (
    error &&
    typeof error === "object" &&
    "graphQLErrors" in error &&
    Array.isArray((error as { graphQLErrors?: unknown[] }).graphQLErrors)
  ) {
    const graphQLErrors = (error as {
      graphQLErrors?: Array<{ extensions?: { code?: unknown } }>;
    }).graphQLErrors;
    const code = graphQLErrors?.find(
      (entry) => typeof entry.extensions?.code === "string",
    )?.extensions?.code;
    return typeof code === "string" ? code : null;
  }

  return null;
}

function loginVerificationFromError(error: unknown): LoginVerificationChallenge | null {
  if (!error || typeof error !== "object" || !("graphQLErrors" in error)) return null;
  const graphQLErrors = (error as {
    graphQLErrors?: Array<{ extensions?: Record<string, unknown> }>;
  }).graphQLErrors;
  const extensions = graphQLErrors?.find(
    (entry) => entry.extensions?.code === "MFA_STEP_UP_REQUIRED",
  )?.extensions;
  if (
    !extensions ||
    typeof extensions.loginChallengeId !== "string" ||
    typeof extensions.expiresAt !== "string" ||
    typeof extensions.hasPasskey !== "boolean" ||
    typeof extensions.hasTotp !== "boolean"
  ) {
    return null;
  }
  return {
    loginChallengeId: extensions.loginChallengeId,
    expiresAt: extensions.expiresAt,
    hasPasskey: extensions.hasPasskey,
    hasTotp: extensions.hasTotp,
  };
}

function primaryLoginFailureMessage(
  t: (key: string) => string,
  error?: unknown,
): string {
  if (error !== undefined && graphQlErrorCode(error) === "RATE_LIMITED") {
    return t("auth.signInRateLimited");
  }
  return t("auth.signInFailedGeneric");
}

const AUTH_PAGE_CLASS =
  "flex min-h-screen items-center justify-center bg-fixed p-4 text-[var(--scry-body)] [background-image:var(--scry-shell-bg)] sm:p-6";
const AUTH_PANEL_CLASS =
  "w-full max-w-sm space-y-5 rounded-[12px] border border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-7 shadow-[0_22px_70px_rgba(2,6,23,0.26)] max-sm:p-5";
const AUTH_MFA_PANEL_CLASS =
  "w-full max-w-md space-y-5 rounded-[12px] border border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-7 shadow-[0_22px_70px_rgba(2,6,23,0.26)] max-sm:p-5";
const AUTH_HEADING_CLASS =
  "text-center font-[var(--font-space-grotesk)] text-2xl font-semibold tracking-normal text-[var(--scry-ink)]";
const AUTH_MUTED_TEXT_CLASS = "text-sm leading-6 text-[var(--scry-muted)]";
const AUTH_LABEL_CLASS = "block text-sm font-medium text-[var(--scry-muted)]";
const AUTH_INPUT_CLASS =
  "h-10 rounded-[9px] border-[var(--scry-border3)] bg-[var(--scry-inset)] text-[var(--scry-ink2)] placeholder:text-[var(--scry-muted3)] focus-visible:border-[var(--scry-accent-ring)] focus-visible:ring-[rgba(var(--scry-accent-rgb),0.25)]";
const AUTH_SELECT_CLASS =
  "h-10 w-full rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 text-sm text-[var(--scry-ink2)] outline-none focus:border-[var(--scry-accent-ring)] focus:ring-2 focus:ring-[rgba(var(--scry-accent-rgb),0.25)]";
const AUTH_PRIMARY_BUTTON_CLASS =
  "flex h-10 w-full items-center justify-center gap-2 rounded-[9px] bg-primary px-4 text-sm font-semibold text-primary-foreground shadow-none transition-colors hover:bg-primary/90 disabled:opacity-50";
const AUTH_SECONDARY_BUTTON_CLASS =
  "flex h-10 w-full items-center justify-center gap-2 rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-4 text-sm font-semibold text-[var(--scry-ink2)] shadow-none transition-colors hover:bg-[var(--scry-hover)] disabled:opacity-50";
const AUTH_ERROR_CLASS =
  "rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm leading-6 text-[var(--scry-danger-text)]";

/// Brand mark stacked over the wordmark at the top of the sign-in card.
///
/// The mark is the 512px icon rather than `scryer-logo.svg`: the SVG carries an
/// embedded raster and weighs 2.6MB, which is a lot to spend on the one page
/// every unauthenticated visitor loads. At 256 CSS px the 512px source is still
/// pixel-doubled on retina.
///
/// Only one wordmark asset exists and its letterforms are light ink for a dark
/// background, so on the light theme it is darkened to stay legible. The theme
/// is read after mount for the same reason the sidebar does it: `resolvedTheme`
/// is undefined on the first pass and the mark would otherwise flip after paint.
function AuthBrand() {
  const { resolvedTheme } = useTheme();
  const [themeMounted, setThemeMounted] = useState(false);
  useEffect(() => setThemeMounted(true), []);
  const lightTheme = themeMounted && resolvedTheme === "light";

  return (
    <div className="flex flex-col items-center gap-3">
      <img
        id="login-brand-logo"
        src={`${import.meta.env.BASE_URL}scryer-icon-512.png`}
        alt=""
        width={256}
        height={256}
        className="h-auto w-64 max-w-full"
      />
      <img
        id="login-brand-wordmark"
        src={`${import.meta.env.BASE_URL}scryer-wordmark.svg`}
        alt="Scryer"
        className={cn(
          "h-auto w-56 max-w-full",
          lightTheme && "[filter:brightness(0.2)_saturate(1.15)]",
        )}
      />
    </div>
  );
}

export default function LoginPage() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const { serviceRestarting } = useBackendRestarting();
  const { t } = useLanguage(searchParams);
  const {
    login,
    adoptSession,
    logout,
    user,
    passwordChangeRequired,
    loading: authLoading,
    effectiveFormLoginEnabled,
    passkeyEnabled,
    defaultPersistSession,
  } = useAuth();
  // Default to the Scryer password method so its form is in the DOM and visible
  // at first paint. Password managers detect credential fields on load; a form
  // that only mounts after a chooser click is never offered for autofill. This
  // also pins the form across the async settings load below, which would
  // otherwise raise the method count and unmount an already-detected form.
  const [activeMethod, setActiveMethod] = useState<LoginMethod>("password");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [replacementPassword, setReplacementPassword] = useState("");
  const [replacementPasswordConfirmation, setReplacementPasswordConfirmation] =
    useState("");
  const [showReplacementPassword, setShowReplacementPassword] = useState(false);
  const replacementPasswordInput = useRef<HTMLInputElement | null>(null);
  const [persistSession, setPersistSession] = useState(false);
  const persistSessionInitialized = useRef(false);
  const verificationPasskeyAbort = useRef<AbortController | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [loginVerification, setLoginVerification] =
    useState<LoginVerificationChallenge | null>(null);
  const [verificationFactor, setVerificationFactor] = useState<"passkey" | "totp">("passkey");
  const [verificationTotpCode, setVerificationTotpCode] = useState("");
  const [verificationPasskeyBusy, setVerificationPasskeyBusy] = useState(false);
  const [verificationPasskeyStatus, setVerificationPasskeyStatus] = useState<
    "waiting" | "cancelled" | "failed"
  >("waiting");
  const [verificationPasskeyAttempt, setVerificationPasskeyAttempt] = useState(0);
  const [passkeySubmitting, setPasskeySubmitting] = useState(false);
  const [jellyfinSubmitting, setJellyfinSubmitting] = useState(false);
  const [embySubmitting, setEmbySubmitting] = useState(false);
  const [externalAuthSettings, setExternalAuthSettings] =
    useState<ExternalAuthRuntimeSettings | null>(null);
  const [jellyfinConnectionId, setJellyfinConnectionId] = useState("");
  const [embyConnectionId, setEmbyConnectionId] = useState("");
  const [embyMode, setEmbyMode] = useState<"LOCAL" | "CONNECT">("LOCAL");
  const [embyUsername, setEmbyUsername] = useState("");
  const [embyPassword, setEmbyPassword] = useState("");
  const [plexConnectionId, setPlexConnectionId] = useState("");
  const [jellyfinUsername, setJellyfinUsername] = useState("");
  const [jellyfinPassword, setJellyfinPassword] = useState("");
  const [jellyfinMfaSetupActive, setJellyfinMfaSetupActive] = useState(false);
  const [jellyfinMfaEnrollment, setJellyfinMfaEnrollment] =
    useState<TotpEnrollmentStart | null>(null);
  const [jellyfinMfaEnrollmentCode, setJellyfinMfaEnrollmentCode] = useState("");
  const [jellyfinMfaRecoveryCodes, setJellyfinMfaRecoveryCodes] = useState<string[]>([]);
  const [jellyfinMfaBusy, setJellyfinMfaBusy] = useState(false);
  const [loginEnrollmentPasskeyBusy, setLoginEnrollmentPasskeyBusy] = useState(false);
  const [plexSubmitting, setPlexSubmitting] = useState(false);
  const redirectTarget = resolveRedirectTarget(searchParams.get("redirect"));

  useEffect(() => {
    if (!authLoading && !persistSessionInitialized.current) {
      setPersistSession(defaultPersistSession);
      persistSessionInitialized.current = true;
    }
  }, [authLoading, defaultPersistSession]);

  useEffect(() => {
    if (passwordChangeRequired) {
      replacementPasswordInput.current?.focus();
    }
  }, [passwordChangeRequired]);
  const jellyfinConnections =
    externalAuthSettings?.loginProviders.includes("JELLYFIN")
      ? externalAuthSettings.connections.filter(
          (connection) => connection.provider === "JELLYFIN" && connection.loginEnabled,
        )
      : [];
  const embyConnections =
    externalAuthSettings?.loginProviders.includes("EMBY")
      ? externalAuthSettings.connections.filter(
          (connection) => connection.provider === "EMBY" && connection.loginEnabled,
        )
      : [];
  const plexConnections =
    isVisibleExternalAccountProvider("PLEX") &&
    externalAuthSettings?.loginProviders.includes("PLEX")
      ? externalAuthSettings.connections.filter(
          (connection) => connection.provider === "PLEX" && connection.loginEnabled,
        )
      : [];
  const plexLoginAvailable = plexConnections.length > 0;
  const localPasswordAvailable = effectiveFormLoginEnabled !== false;
  const jellyfinLoginAvailable = jellyfinConnections.length > 0;
  const embyLoginAvailable = embyConnections.length > 0;
  const loginMethodCount = [
    localPasswordAvailable,
    jellyfinLoginAvailable,
    embyLoginAvailable,
    plexLoginAvailable,
  ].filter(Boolean).length;
  const showLoginMethodChooser = loginMethodCount > 1;
  const passwordFormVisible =
    activeMethod === "password" ||
    (!showLoginMethodChooser && localPasswordAvailable);
  const jellyfinFormVisible =
    activeMethod === "jellyfin" ||
    (!showLoginMethodChooser && jellyfinLoginAvailable);
  const embyFormVisible =
    activeMethod === "emby" || (!showLoginMethodChooser && embyLoginAvailable);
  const selectedEmbyConnection = embyConnections.find(
    (connection) => connection.id === embyConnectionId,
  );
  const anySubmitting =
    submitting ||
    passkeySubmitting ||
    jellyfinSubmitting ||
    embySubmitting ||
    jellyfinMfaBusy ||
    loginEnrollmentPasskeyBusy ||
    plexSubmitting;

  const beginLoginVerification = useCallback((error: unknown): boolean => {
    const challenge = loginVerificationFromError(error);
    if (!challenge) return false;
    setPassword("");
    setJellyfinPassword("");
    setEmbyPassword("");
    setLoginVerification(challenge);
    setVerificationFactor(challenge.hasPasskey ? "passkey" : "totp");
    setVerificationTotpCode("");
    setVerificationPasskeyStatus("waiting");
    setError(null);
    return true;
  }, []);

  const cancelLoginVerification = useCallback(() => {
    verificationPasskeyAbort.current?.abort();
    setLoginVerification(null);
    setVerificationTotpCode("");
    setError(null);
  }, []);

  const adoptCompletedLogin = useCallback(
    (result: CompletedLoginPayload): boolean => {
      adoptSession(result.token, result.user ?? null, result.persistSession);
      if (result.passwordChangeRequired) {
        setPassword("");
        setJellyfinPassword("");
        setEmbyPassword("");
        return true;
      }
      return false;
    },
    [adoptSession],
  );

  useEffect(() => {
    if (!loginVerification) return;
    const delay = new Date(loginVerification.expiresAt).getTime() - Date.now();
    if (delay <= 0) {
      setLoginVerification(null);
      setError("Verification expired. Sign in again to continue.");
      return;
    }
    const timeout = window.setTimeout(() => {
      verificationPasskeyAbort.current?.abort();
      setLoginVerification(null);
      setError("Verification expired. Sign in again to continue.");
    }, delay);
    return () => window.clearTimeout(timeout);
  }, [loginVerification]);

  useEffect(() => {
    if (
      !loginVerification ||
      verificationFactor !== "passkey" ||
      !loginVerification.hasPasskey
    ) {
      return;
    }
    const controller = new AbortController();
    verificationPasskeyAbort.current = controller;
    setVerificationPasskeyBusy(true);
    setVerificationPasskeyStatus("waiting");
    void (async () => {
      try {
        const result = await authenticateLoginVerificationPasskey(
          loginVerification.loginChallengeId,
          controller.signal,
        );
        if (controller.signal.aborted) return;
        if (!adoptCompletedLogin(result)) {
          navigate(redirectTarget, { replace: true });
        }
      } catch (err) {
        if (controller.signal.aborted) return;
        if (err instanceof PasskeyClientError && err.code === "cancelled") {
          setVerificationPasskeyStatus("cancelled");
        } else {
          setVerificationPasskeyStatus("failed");
          setError("Passkey verification could not be completed. Try again or use another factor.");
        }
      } finally {
        if (!controller.signal.aborted) setVerificationPasskeyBusy(false);
      }
    })();
    return () => controller.abort();
  }, [
    adoptCompletedLogin,
    loginVerification,
    navigate,
    redirectTarget,
    verificationFactor,
    verificationPasskeyAttempt,
  ]);

  const handleVerificationTotp = useCallback(async () => {
    if (!loginVerification || !verificationTotpCode.trim()) return;
    setError(null);
    setSubmitting(true);
    try {
      const { data, error } = await backendClient
        .mutation<
          { loginVerificationTotpComplete?: LoginPayload },
          { input: { loginChallengeId: string; code: string } }
        >(loginVerificationTotpCompleteMutation, {
          input: {
            loginChallengeId: loginVerification.loginChallengeId,
            code: verificationTotpCode,
          },
        })
        .toPromise();
      if (error || !data?.loginVerificationTotpComplete) {
        throw error ?? new Error("Authenticator verification failed.");
      }
      const result = data.loginVerificationTotpComplete;
      if (!adoptCompletedLogin(result)) {
        navigate(redirectTarget, { replace: true });
      }
    } catch {
      setError("Authenticator or recovery code was not accepted.");
    } finally {
      setSubmitting(false);
    }
  }, [
    adoptCompletedLogin,
    loginVerification,
    navigate,
    redirectTarget,
    verificationTotpCode,
  ]);

  // Redirect to home if already authenticated
  useEffect(() => {
    if (!serviceRestarting && !authLoading && user && !jellyfinMfaSetupActive) {
      navigate(redirectTarget, { replace: true });
    }
  }, [
    authLoading,
    jellyfinMfaSetupActive,
    user,
    navigate,
    redirectTarget,
    serviceRestarting,
  ]);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const { data, error } = await backendClient
          .query<{ externalAuthRuntimeSettings?: ExternalAuthRuntimeSettings }>(
            externalAuthRuntimeSettingsQuery,
            {},
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error || cancelled) return;
        const settings = data?.externalAuthRuntimeSettings ?? null;
        setExternalAuthSettings(settings);
        const firstJellyfinConnectionId =
          settings?.connections.find(
            (connection) => connection.provider === "JELLYFIN" && connection.loginEnabled,
          )?.id ??
          "";
        if (firstJellyfinConnectionId) {
          setJellyfinConnectionId((current) =>
            current || firstJellyfinConnectionId,
          );
        }
        const firstEmbyConnectionId =
          settings?.connections.find(
            (connection) => connection.provider === "EMBY" && connection.loginEnabled,
          )?.id ?? "";
        if (firstEmbyConnectionId) {
          setEmbyConnectionId((current) => current || firstEmbyConnectionId);
        }
        if (isVisibleExternalAccountProvider("PLEX")) {
          const firstPlexConnectionId =
            settings?.connections.find(
              (connection) => connection.provider === "PLEX" && connection.loginEnabled,
            )?.id ??
            "";
          if (firstPlexConnectionId) {
            setPlexConnectionId((current) => current || firstPlexConnectionId);
          }
        }
      } catch {
        // Provider login remains hidden when settings cannot be loaded.
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  const handlePasskeySignIn = useCallback(
    async () => {
      setError(null);
      setPasskeySubmitting(true);
      try {
        const result = await authenticateWithPasskey(undefined, persistSession);
        adoptSession(result.token, result.user, result.persistSession);
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        if (err instanceof PasskeyClientError) {
          if (err.code === "unsupported") {
            setError(t("auth.passkeyUnsupported"));
          } else if (err.code === "cancelled") {
            setError(t("auth.passkeyCancelled"));
          } else {
            setError(primaryLoginFailureMessage(t, err));
          }
          return;
        }

        setError(primaryLoginFailureMessage(t, err));
      } finally {
        setPasskeySubmitting(false);
      }
    },
    [adoptSession, navigate, persistSession, redirectTarget, t],
  );

  const startJellyfinMfaEnrollment = useCallback(async () => {
    setJellyfinMfaBusy(true);
    setJellyfinMfaRecoveryCodes([]);
    setJellyfinMfaEnrollmentCode("");
    try {
      const { data, error } = await mfaEnrollmentClient
        .mutation<{ totpEnrollmentStart?: TotpEnrollmentStart }>(
          totpEnrollmentStartMutation,
          {},
        )
        .toPromise();
      if (error || !data?.totpEnrollmentStart) {
        throw error ?? new Error(t("auth.mfaSetupStartFailed"));
      }
      setJellyfinMfaEnrollment(data.totpEnrollmentStart);
    } catch (err) {
      setError(err instanceof Error ? err.message : t("auth.mfaSetupStartFailed"));
    } finally {
      setJellyfinMfaBusy(false);
    }
  }, [t]);

  const completeLoginEnrollmentWithPasskey = useCallback(async () => {
    setError(null);
    setLoginEnrollmentPasskeyBusy(true);
    try {
      const result = await registerLoginEnrollmentPasskey(mfaEnrollmentClient);
      const passwordReplacementPending = adoptCompletedLogin(result.login);
      setJellyfinMfaSetupActive(false);
      if (!passwordReplacementPending) {
        navigate(redirectTarget, { replace: true });
      }
    } catch (err) {
      if (err instanceof PasskeyClientError && err.code === "cancelled") {
        setError("Passkey enrollment was cancelled. Choose an authenticator app instead or try again.");
      } else {
        setError("Passkey enrollment could not be completed.");
      }
    } finally {
      setLoginEnrollmentPasskeyBusy(false);
    }
  }, [adoptCompletedLogin, navigate, redirectTarget]);

  const handleSubmit = useCallback(
    async (e?: React.FormEvent) => {
      e?.preventDefault();
      setError(null);
      setSubmitting(true);
      try {
        const result = await login(username, password, {
          totpCode: null,
          persistSession,
        });
        if (result.mfaEnrollmentRequired) {
          setPassword("");
          setJellyfinMfaSetupActive(true);
          adoptSession(result.token, result.user ?? null, result.persistSession);
          return;
        }
        if (result.passwordChangeRequired) {
          setPassword("");
          return;
        }
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        if (!beginLoginVerification(err)) {
          setError(primaryLoginFailureMessage(t, err));
        }
      } finally {
        setSubmitting(false);
      }
    },
    [
      adoptSession,
      beginLoginVerification,
      login,
      navigate,
      password,
      persistSession,
      redirectTarget,
      t,
      username,
    ],
  );

  const completeJellyfinMfaEnrollment = useCallback(async () => {
    if (!jellyfinMfaEnrollment || jellyfinMfaEnrollmentCode.length !== 6) return;

    setError(null);
    setJellyfinMfaBusy(true);
    try {
      const { data, error } = await mfaEnrollmentClient
        .mutation<
          {
            completeLoginMfaEnrollment?: TotpEnrollmentComplete & {
              login: LoginPayload;
            };
          },
          { input: { challengeId: string; code: string } }
        >(completeLoginMfaEnrollmentMutation, {
          input: {
            challengeId: jellyfinMfaEnrollment.challengeId,
            code: jellyfinMfaEnrollmentCode,
          },
        })
        .toPromise();
      if (error || !data?.completeLoginMfaEnrollment) {
        throw error ?? new Error(t("auth.mfaSetupCompleteFailed"));
      }
      setJellyfinMfaRecoveryCodes(data.completeLoginMfaEnrollment.recoveryCodes);
      setJellyfinMfaEnrollment(null);
      setJellyfinMfaEnrollmentCode("");
      adoptCompletedLogin(data.completeLoginMfaEnrollment.login);
    } catch (err) {
      setError(err instanceof Error ? err.message : t("auth.mfaSetupCompleteFailed"));
    } finally {
      setJellyfinMfaBusy(false);
    }
  }, [
    adoptCompletedLogin,
    jellyfinMfaEnrollment,
    jellyfinMfaEnrollmentCode,
    t,
  ]);

  const continueAfterJellyfinMfaEnrollment = useCallback(() => {
    setJellyfinMfaSetupActive(false);
    if (!passwordChangeRequired) {
      navigate(redirectTarget, { replace: true });
    }
  }, [navigate, passwordChangeRequired, redirectTarget]);

  const cancelJellyfinMfaEnrollment = useCallback(() => {
    logout();
    setJellyfinMfaSetupActive(false);
    setJellyfinMfaEnrollment(null);
    setJellyfinMfaEnrollmentCode("");
    setJellyfinMfaRecoveryCodes([]);
    setJellyfinPassword("");
    setError(null);
  }, [logout]);

  const handleJellyfinSignIn = useCallback(
    async () => {
      if (!jellyfinConnectionId || !jellyfinUsername || !jellyfinPassword) return;

      setError(null);
      setJellyfinSubmitting(true);
      try {
        const { data, error } = await backendClient
          .mutation(loginWithJellyfinMutation, {
            input: {
              connectionId: jellyfinConnectionId,
              username: jellyfinUsername,
              password: jellyfinPassword,
              totpCode: null,
              persistSession,
            },
          })
          .toPromise();
        if (error || !data?.loginWithJellyfin) {
          throw error ?? new Error(t("auth.jellyfinFailed"));
        }
        const loginPayload = data.loginWithJellyfin;
        if (loginPayload.mfaEnrollmentRequired) {
          setJellyfinMfaSetupActive(true);
          adoptSession(
            loginPayload.token,
            loginPayload.user ?? null,
            loginPayload.persistSession,
          );
          return;
        }
        adoptSession(
          loginPayload.token,
          loginPayload.user ?? null,
          loginPayload.persistSession,
        );
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        if (!beginLoginVerification(err)) {
          setError(primaryLoginFailureMessage(t));
        }
      } finally {
        setJellyfinSubmitting(false);
      }
    },
    [
      adoptSession,
      beginLoginVerification,
      jellyfinConnectionId,
      jellyfinPassword,
      jellyfinUsername,
      navigate,
      persistSession,
      redirectTarget,
      t,
    ],
  );

  const handleEmbySignIn = useCallback(async () => {
    if (!embyConnectionId || !embyUsername || !embyPassword) return;

    setError(null);
    setEmbySubmitting(true);
    try {
      const { data, error } = await backendClient
        .mutation(loginWithEmbyMutation, {
          input: {
            connectionId: embyConnectionId,
            mode: embyMode,
            username: embyUsername,
            password: embyPassword,
            totpCode: null,
            persistSession,
          },
        })
        .toPromise();
      if (error || !data?.loginWithEmby) {
        throw error ?? new Error("Emby sign-in failed");
      }
      const loginPayload = data.loginWithEmby;
      if (loginPayload.mfaEnrollmentRequired) {
        setJellyfinMfaSetupActive(true);
        adoptSession(
          loginPayload.token,
          loginPayload.user ?? null,
          loginPayload.persistSession,
        );
        return;
      }
      adoptSession(
        loginPayload.token,
        loginPayload.user ?? null,
        loginPayload.persistSession,
      );
      navigate(redirectTarget, { replace: true });
    } catch (err) {
      if (!beginLoginVerification(err)) {
        setError("Emby sign-in failed");
      }
    } finally {
      setEmbySubmitting(false);
    }
  }, [
    adoptSession,
    beginLoginVerification,
    embyConnectionId,
    embyMode,
    embyPassword,
    embyUsername,
    navigate,
    persistSession,
    redirectTarget,
  ]);

  const handlePlexSignIn = useCallback(
    async () => {
      if (!plexConnectionId) return;

      setError(null);
      setPlexSubmitting(true);
      try {
        const plexAuthToken = await authenticateWithPlexPin();
        const { data, error } = await backendClient
          .mutation(loginWithPlexMutation, {
            input: {
              connectionId: plexConnectionId,
              plexAuthToken,
              persistSession,
            },
          })
          .toPromise();
        if (error || !data?.loginWithPlex) {
          throw error ?? new Error(t("auth.plexFailed"));
        }
        adoptSession(
          data.loginWithPlex.token,
          data.loginWithPlex.user ?? null,
          data.loginWithPlex.persistSession,
        );
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        setError(primaryLoginFailureMessage(t, err));
      } finally {
        setPlexSubmitting(false);
      }
    },
    [adoptSession, navigate, persistSession, plexConnectionId, redirectTarget, t],
  );

  const handleRequiredPasswordChange = useCallback(
    async (event: React.FormEvent) => {
      event.preventDefault();
      if (!replacementPassword) {
        setError("Enter a new password.");
        replacementPasswordInput.current?.focus();
        return;
      }
      if (replacementPassword !== replacementPasswordConfirmation) {
        setError("The new passwords do not match.");
        return;
      }

      setError(null);
      setSubmitting(true);
      try {
        const { data, error } = await passwordChangeRequiredClient
          .mutation<{ completeRequiredPasswordChange?: LoginPayload }>(
            completeRequiredPasswordChangeMutation,
            { input: { password: replacementPassword } },
          )
          .toPromise();
        if (error || !data?.completeRequiredPasswordChange) {
          throw error ?? new Error("Password replacement failed.");
        }
        const result = data.completeRequiredPasswordChange;
        adoptSession(result.token, result.user ?? null, result.persistSession);
        setReplacementPassword("");
        setReplacementPasswordConfirmation("");
        navigate(redirectTarget, { replace: true });
      } catch (err) {
        if (graphQlErrorCode(err) === "PASSWORD_CHANGE_REQUIRED") {
          logout();
          setError("This temporary session has expired. Sign in again to continue.");
          return;
        }
        setError("Your new password could not be saved. Check the requirements and try again.");
      } finally {
        setSubmitting(false);
      }
    },
    [
      adoptSession,
      logout,
      navigate,
      redirectTarget,
      replacementPassword,
      replacementPasswordConfirmation,
    ],
  );

  if (serviceRestarting) {
    return <BackendRestartOverlay />;
  }

  if (authLoading) {
    return (
      <div className={AUTH_PAGE_CLASS}>
        <Loader2 className="h-6 w-6 animate-spin text-[var(--scry-accent-ring)]" />
      </div>
    );
  }

  if (loginVerification) {
    const passkeyOnly = loginVerification.hasPasskey && !loginVerification.hasTotp;
    return (
      <div className={AUTH_PAGE_CLASS}>
        <div className={AUTH_MFA_PANEL_CLASS}>
          <div className="space-y-2 text-center">
            <h1 className={AUTH_HEADING_CLASS}>Confirm it&apos;s you</h1>
            <p className={AUTH_MUTED_TEXT_CLASS}>
              Complete an enrolled authentication factor to finish signing in.
            </p>
          </div>

          <div aria-live="polite" className="sr-only">
            {verificationFactor === "passkey" && verificationPasskeyBusy
              ? "Waiting for your passkey."
              : verificationPasskeyStatus === "cancelled"
                ? "Passkey request was cancelled."
                : verificationPasskeyStatus === "failed"
                  ? "Passkey verification failed."
                  : ""}
          </div>

          {error ? <div id="login-error" className={AUTH_ERROR_CLASS}>{error}</div> : null}

          {verificationFactor === "totp" ? (
            <TotpCodeForm
              id="login-verification-form"
              inputId="login-verification-totp-code"
              submitId="login-verification-totp-submit"
              code={verificationTotpCode}
              title="Authenticator or recovery code"
              description="Enter a code from your authenticator app or a recovery code."
              submitLabel="Verify"
              busyLabel="Verifying"
              cancelLabel="Back"
              busy={submitting}
              disabled={submitting}
              allowRecoveryCode
              onCodeChange={setVerificationTotpCode}
              onSubmit={() => void handleVerificationTotp()}
              onCancel={cancelLoginVerification}
            />
          ) : (
            <div className="space-y-3">
              <p className={AUTH_MUTED_TEXT_CLASS}>
                {verificationPasskeyBusy
                  ? "Waiting for your passkey…"
                  : verificationPasskeyStatus === "cancelled"
                    ? "Your passkey request was cancelled."
                    : "Use your passkey to continue."}
              </p>
              <button
                id="login-verification-passkey-retry"
                type="button"
                onClick={() => {
                  setError(null);
                  setVerificationPasskeyAttempt((attempt) => attempt + 1);
                }}
                disabled={verificationPasskeyBusy}
                className={AUTH_PRIMARY_BUTTON_CLASS}
              >
                {verificationPasskeyBusy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Fingerprint className="h-4 w-4" aria-hidden="true" />}
                {verificationPasskeyBusy ? "Waiting for passkey" : "Try passkey again"}
              </button>
              {loginVerification.hasTotp ? (
                <button
                  id="login-verification-use-totp"
                  type="button"
                  onClick={() => {
                    verificationPasskeyAbort.current?.abort();
                    setVerificationFactor("totp");
                    setError(null);
                  }}
                  className={AUTH_SECONDARY_BUTTON_CLASS}
                >
                  Use an authenticator or recovery code instead
                </button>
              ) : null}
              {passkeyOnly && !verificationPasskeyBusy ? (
                <p className={AUTH_MUTED_TEXT_CLASS}>
                  Can&apos;t use your passkey? Contact an administrator to recover your account.
                </p>
              ) : null}
              <button
                type="button"
                onClick={cancelLoginVerification}
                disabled={verificationPasskeyBusy}
                className={AUTH_SECONDARY_BUTTON_CLASS}
              >
                Back to sign in
              </button>
            </div>
          )}
        </div>
      </div>
    );
  }

  if (jellyfinMfaSetupActive) {
    return (
      <div className={AUTH_PAGE_CLASS}>
        <div className={AUTH_MFA_PANEL_CLASS}>
          <div className="space-y-2 text-center">
            <h1 className={AUTH_HEADING_CLASS}>{t("auth.mfaSetupTitle")}</h1>
            <p className={AUTH_MUTED_TEXT_CLASS}>{t("auth.mfaSetupDescription")}</p>
          </div>

          {error ? (
            <div id="login-error" className={AUTH_ERROR_CLASS}>
              {error}
            </div>
          ) : null}

          {jellyfinMfaRecoveryCodes.length > 0 ? (
            <div className="space-y-4">
              <p className={AUTH_MUTED_TEXT_CLASS}>
                {t("auth.mfaRecoveryCodesDescription")}
              </p>
              <div className="grid grid-cols-2 gap-2 rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-3 font-[var(--font-code)] text-xs text-[var(--scry-ink2)]">
                {jellyfinMfaRecoveryCodes.map((code) => (
                  <code key={code}>{code}</code>
                ))}
              </div>
              <button
                id="jellyfin-mfa-enrollment-continue"
                type="button"
                onClick={continueAfterJellyfinMfaEnrollment}
                className={AUTH_PRIMARY_BUTTON_CLASS}
              >
                {t("auth.continue")}
              </button>
            </div>
          ) : jellyfinMfaEnrollment ? (
            <div className="space-y-4">
              <div className="flex flex-col items-center gap-4">
                <TotpQrCode
                  id="jellyfin-mfa-enrollment-qr-code"
                  value={jellyfinMfaEnrollment.otpauthUrl}
                />
                <a
                  id="jellyfin-mfa-enrollment-setup-link"
                  className="break-all text-sm font-medium text-[var(--scry-accent-text)] underline-offset-4 hover:underline"
                  href={jellyfinMfaEnrollment.otpauthUrl}
                >
                  {t("profile.totpOpenSetupLink")}
                </a>
                <div className="w-full space-y-1">
                  <div className="text-xs text-[var(--scry-muted)]">{t("profile.totpSecret")}</div>
                  <code
                    id="jellyfin-mfa-enrollment-secret"
                    className="block break-all rounded-[7px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-1 font-[var(--font-code)] text-xs text-[var(--scry-ink2)]"
                  >
                    {jellyfinMfaEnrollment.secretBase32}
                  </code>
                </div>
              </div>
              <div className="space-y-2">
                <Input
                  {...integerInputProps}
                  id="jellyfin-mfa-enrollment-code"
                  autoComplete="one-time-code"
                  maxLength={6}
                  value={jellyfinMfaEnrollmentCode}
                  onChange={(event) => setJellyfinMfaEnrollmentCode(sanitizeTotpCode(event.target.value))}
                  placeholder={t("auth.totpCode")}
                  className={AUTH_INPUT_CLASS}
                />
                <button
                  id="jellyfin-mfa-enrollment-submit"
                  type="button"
                  onClick={completeJellyfinMfaEnrollment}
                  disabled={jellyfinMfaBusy || jellyfinMfaEnrollmentCode.length !== 6}
                  className={AUTH_PRIMARY_BUTTON_CLASS}
                >
                  {jellyfinMfaBusy ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                  {t("profile.totpVerifyAndEnable")}
                </button>
              </div>
              <button
                type="button"
                onClick={cancelJellyfinMfaEnrollment}
                disabled={jellyfinMfaBusy}
                className={AUTH_SECONDARY_BUTTON_CLASS}
              >
                {t("auth.mfaSetupCancel")}
              </button>
            </div>
          ) : (
            <div className="space-y-3">
              <p className={AUTH_MUTED_TEXT_CLASS}>
                Protect your account with a passkey or an authenticator app.
              </p>
              {passkeyEnabled ? (
                <button
                  id="login-mfa-enrollment-passkey"
                  type="button"
                  onClick={() => void completeLoginEnrollmentWithPasskey()}
                  disabled={jellyfinMfaBusy || loginEnrollmentPasskeyBusy}
                  className={AUTH_PRIMARY_BUTTON_CLASS}
                >
                  {loginEnrollmentPasskeyBusy ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <Fingerprint className="h-4 w-4" aria-hidden="true" />
                  )}
                  Create a passkey
                </button>
              ) : null}
              <button
                type="button"
                onClick={() => void startJellyfinMfaEnrollment()}
                disabled={jellyfinMfaBusy || loginEnrollmentPasskeyBusy}
                className={AUTH_SECONDARY_BUTTON_CLASS}
              >
                Use an authenticator app instead
              </button>
              <button
                type="button"
                onClick={cancelJellyfinMfaEnrollment}
                disabled={jellyfinMfaBusy || loginEnrollmentPasskeyBusy}
                className={AUTH_SECONDARY_BUTTON_CLASS}
              >
                {t("auth.mfaSetupCancel")}
              </button>
            </div>
          )}
        </div>
      </div>
    );
  }

  if (passwordChangeRequired) {
    return (
      <div className={AUTH_PAGE_CLASS}>
        <div className={AUTH_PANEL_CLASS}>
          <div className="space-y-2 text-center">
            <h1 className={AUTH_HEADING_CLASS}>Choose a new password</h1>
            <p className={AUTH_MUTED_TEXT_CLASS}>
              Your administrator supplied a temporary password. Choose a new password that only you know.
            </p>
          </div>

          <div aria-live="polite" className="sr-only">
            {error ?? ""}
          </div>
          {error ? <div id="login-error" className={AUTH_ERROR_CLASS}>{error}</div> : null}

          <form onSubmit={handleRequiredPasswordChange} className="space-y-4">
            <div className="space-y-1.5">
              <label htmlFor="required-password" className={AUTH_LABEL_CLASS}>
                New password
              </label>
              <Input
                ref={replacementPasswordInput}
                id="required-password"
                type={showReplacementPassword ? "text" : "password"}
                autoComplete="new-password"
                required
                value={replacementPassword}
                onChange={(event) => setReplacementPassword(event.target.value)}
                className={AUTH_INPUT_CLASS}
              />
            </div>
            <div className="space-y-1.5">
              <label htmlFor="required-password-confirmation" className={AUTH_LABEL_CLASS}>
                Confirm new password
              </label>
              <Input
                id="required-password-confirmation"
                type={showReplacementPassword ? "text" : "password"}
                autoComplete="new-password"
                required
                value={replacementPasswordConfirmation}
                onChange={(event) => setReplacementPasswordConfirmation(event.target.value)}
                className={AUTH_INPUT_CLASS}
              />
            </div>
            <label className="flex items-center gap-2 text-sm text-[var(--scry-muted)]">
              <input
                type="checkbox"
                checked={showReplacementPassword}
                onChange={(event) => setShowReplacementPassword(event.target.checked)}
              />
              Show passwords
            </label>
            <button
              id="complete-required-password-change"
              type="submit"
              disabled={submitting}
              className={AUTH_PRIMARY_BUTTON_CLASS}
            >
              {submitting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {submitting ? "Saving password" : "Save new password"}
            </button>
          </form>

          <button
            type="button"
            onClick={logout}
            disabled={submitting}
            className={AUTH_SECONDARY_BUTTON_CLASS}
          >
            Sign out
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className={AUTH_PAGE_CLASS}>
      <div className={AUTH_PANEL_CLASS}>
        <AuthBrand />
        <h1 className={AUTH_HEADING_CLASS}>{t("auth.signIn")}</h1>

        {error && (
          <div id="login-error" className={AUTH_ERROR_CLASS}>
            {error}
          </div>
        )}

        {!passwordFormVisible ? (
          <label className="flex items-center gap-2 text-sm text-[var(--scry-muted)]">
            <input
              id="persist-session"
              type="checkbox"
              checked={persistSession}
              onChange={(event) => {
                persistSessionInitialized.current = true;
                setPersistSession(event.target.checked);
              }}
              disabled={anySubmitting}
            />
            Keep me signed in
          </label>
        ) : null}

        <div className="space-y-3">
          {localPasswordAvailable ? (
            <>
              {showLoginMethodChooser ? (
                <button
                  id="login-password-method"
                  type="button"
                  onClick={() =>
                    setActiveMethod((current) =>
                      current === "password" ? null : "password",
                    )
                  }
                  disabled={anySubmitting}
                  aria-controls="login-form"
                  aria-expanded={activeMethod === "password"}
                  className={AUTH_SECONDARY_BUTTON_CLASS}
                >
                  <KeyRound className="h-4 w-4" aria-hidden="true" />
                  {t("auth.signInWithScryerPassword")}
                </button>
              ) : null}

              {passwordFormVisible ? (
                <form id="login-form" onSubmit={handleSubmit} className="space-y-5">
                    <div className="space-y-1.5">
                      <label
                        htmlFor="username"
                        className={AUTH_LABEL_CLASS}
                      >
                        {t("auth.username")}
                      </label>
                      <Input
                        id="username"
                        type="text"
                        autoComplete="username"
                        required
                        value={username}
                        onChange={(e) => setUsername(e.target.value)}
                        placeholder={t("auth.username")}
                        className={AUTH_INPUT_CLASS}
                      />
                    </div>

                    <div className="space-y-1.5">
                      <label
                        htmlFor="password"
                        className={AUTH_LABEL_CLASS}
                      >
                        {t("auth.password")}
                      </label>
                      <Input
                        id="password"
                        type="password"
                        autoComplete="current-password"
                        required
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                        placeholder={t("auth.password")}
                        className={AUTH_INPUT_CLASS}
                      />
                    </div>

                    <label className="flex items-center gap-2 text-sm text-[var(--scry-muted)]">
                      <input
                        id="persist-session"
                        type="checkbox"
                        checked={persistSession}
                        onChange={(event) => {
                          persistSessionInitialized.current = true;
                          setPersistSession(event.target.checked);
                        }}
                        disabled={anySubmitting}
                      />
                      Keep me signed in
                    </label>

                    <button
                      id="login-submit"
                      type="submit"
                      disabled={anySubmitting}
                      className={AUTH_PRIMARY_BUTTON_CLASS}
                    >
                      {submitting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                      {submitting ? t("auth.signingIn") : t("auth.signIn")}
                    </button>
                </form>
              ) : null}
            </>
          ) : null}

          {passkeyEnabled ? (
            <button
              id="login-passkey-submit"
              type="button"
              onClick={handlePasskeySignIn}
              disabled={anySubmitting}
              className={AUTH_SECONDARY_BUTTON_CLASS}
            >
              {passkeySubmitting ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Fingerprint className="h-4 w-4" aria-hidden="true" />
              )}
              {passkeySubmitting ? t("auth.passkeySigningIn") : "Sign in with a passkey"}
            </button>
          ) : null}

          {jellyfinLoginAvailable ? (
            <>
              {showLoginMethodChooser ? (
                <button
                  id="login-jellyfin-method"
                  type="button"
                  onClick={() =>
                    setActiveMethod((current) =>
                      current === "jellyfin" ? null : "jellyfin",
                    )
                  }
                  disabled={anySubmitting}
                  aria-controls="jellyfin-login-form"
                  aria-expanded={activeMethod === "jellyfin"}
                  className={AUTH_SECONDARY_BUTTON_CLASS}
                >
                  <img
                    src="/auth-providers/jellyfin.svg"
                    alt=""
                    aria-hidden="true"
                    className="h-4 w-4"
                  />
                  {t("auth.signInWithJellyfin")}
                </button>
              ) : null}

              {jellyfinFormVisible ? (
                // Jellyfin credentials are a separate account from the Scryer
                // login, so this form opts out of password-manager autofill and
                // save prompts. It is still a real form so Enter submits.
                <form
                  id="jellyfin-login-form"
                  className="space-y-3"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void handleJellyfinSignIn();
                  }}
                >
                  {jellyfinConnections.length > 1 ? (
                    <select
                      id="login-jellyfin-connection"
                      className={AUTH_SELECT_CLASS}
                      value={jellyfinConnectionId}
                      onChange={(event) => setJellyfinConnectionId(event.target.value)}
                    >
                      {jellyfinConnections.map((connection) => (
                        <option
                          id={selectorId(
                            "login-jellyfin-connection-option",
                            connection.id,
                          )}
                          key={connection.id}
                          value={connection.id}
                        >
                          {connectionOptionLabel(connection)}
                        </option>
                      ))}
                    </select>
                  ) : null}
                  <Input
                    id="jellyfin-username"
                    type="text"
                    ignorePasswordManagers
                    value={jellyfinUsername}
                    onChange={(event) => setJellyfinUsername(event.target.value)}
                    placeholder={t("auth.username")}
                    className={AUTH_INPUT_CLASS}
                  />
                  <Input
                    id="jellyfin-password"
                    type="password"
                    ignorePasswordManagers
                    value={jellyfinPassword}
                    onChange={(event) => setJellyfinPassword(event.target.value)}
                    placeholder={t("auth.password")}
                    className={AUTH_INPUT_CLASS}
                  />
                  <button
                    id="jellyfin-login-submit"
                    type="submit"
                    disabled={
                      anySubmitting ||
                      !jellyfinConnectionId ||
                      !jellyfinUsername ||
                      !jellyfinPassword
                    }
                    className={AUTH_PRIMARY_BUTTON_CLASS}
                  >
                    {jellyfinSubmitting ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : null}
                    {jellyfinSubmitting ? t("auth.signingIn") : t("auth.signIn")}
                  </button>
                </form>
              ) : null}
            </>
          ) : null}

          {embyLoginAvailable ? (
            <>
              {showLoginMethodChooser ? (
                <button
                  id="login-emby-method"
                  type="button"
                  onClick={() =>
                    setActiveMethod((current) => (current === "emby" ? null : "emby"))
                  }
                  disabled={anySubmitting}
                  aria-controls="emby-login-form"
                  aria-expanded={activeMethod === "emby"}
                  className={AUTH_SECONDARY_BUTTON_CLASS}
                >
                  <img
                    src="/auth-providers/emby.svg"
                    alt=""
                    aria-hidden="true"
                    className="h-4 w-4"
                  />
                  Sign in with Emby
                </button>
              ) : null}

              {embyFormVisible ? (
                <form
                  id="emby-login-form"
                  className="space-y-3"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void handleEmbySignIn();
                  }}
                >
                  <select
                    id="login-emby-connection"
                    className={AUTH_SELECT_CLASS}
                    value={embyConnectionId}
                    onChange={(event) => {
                      const nextId = event.target.value;
                      const nextConnection = embyConnections.find(
                        (connection) => connection.id === nextId,
                      );
                      setEmbyConnectionId(nextId);
                      if (!nextConnection?.embyConnectEnabled) setEmbyMode("LOCAL");
                    }}
                  >
                    {embyConnections.map((connection) => (
                      <option
                        id={selectorId("login-emby-connection-option", connection.id)}
                        key={connection.id}
                        value={connection.id}
                      >
                        {connectionOptionLabel(connection)}
                      </option>
                    ))}
                  </select>
                  {selectedEmbyConnection?.embyConnectEnabled ? (
                    <div className="grid grid-cols-2 gap-2">
                      <button
                        id="login-emby-mode-local"
                        type="button"
                        aria-pressed={embyMode === "LOCAL"}
                        onClick={() => setEmbyMode("LOCAL")}
                        className={AUTH_SECONDARY_BUTTON_CLASS}
                      >
                        Local
                      </button>
                      <button
                        id="login-emby-mode-connect"
                        type="button"
                        aria-pressed={embyMode === "CONNECT"}
                        onClick={() => setEmbyMode("CONNECT")}
                        className={AUTH_SECONDARY_BUTTON_CLASS}
                      >
                        Connect
                      </button>
                    </div>
                  ) : null}
                  <Input
                    id="login-emby-username"
                    type="text"
                    ignorePasswordManagers
                    value={embyUsername}
                    onChange={(event) => setEmbyUsername(event.target.value)}
                    placeholder={
                      embyMode === "CONNECT" ? "Emby Connect username or email" : t("auth.username")
                    }
                    className={AUTH_INPUT_CLASS}
                  />
                  <Input
                    id="login-emby-password"
                    type="password"
                    ignorePasswordManagers
                    value={embyPassword}
                    onChange={(event) => setEmbyPassword(event.target.value)}
                    placeholder={t("auth.password")}
                    className={AUTH_INPUT_CLASS}
                  />
                  <button
                    id="login-emby-submit"
                    type="submit"
                    disabled={anySubmitting || !embyConnectionId || !embyUsername || !embyPassword}
                    className={AUTH_PRIMARY_BUTTON_CLASS}
                  >
                    {embySubmitting ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                    {embySubmitting ? t("auth.signingIn") : t("auth.signIn")}
                  </button>
                </form>
              ) : null}
            </>
          ) : null}

          {plexLoginAvailable ? (
            <div className="space-y-3">
              {plexConnections.length > 1 ? (
                <select
                  id="login-plex-connection"
                  className={AUTH_SELECT_CLASS}
                  value={plexConnectionId}
                  onChange={(event) => setPlexConnectionId(event.target.value)}
                >
                  {plexConnections.map((connection) => (
                    <option
                      id={selectorId("login-plex-connection-option", connection.id)}
                      key={connection.id}
                      value={connection.id}
                    >
                      {connectionOptionLabel(connection)}
                    </option>
                  ))}
                </select>
              ) : null}
              <button
                id="login-plex-submit"
                type="button"
                onClick={handlePlexSignIn}
                disabled={anySubmitting || !plexConnectionId}
                className={AUTH_SECONDARY_BUTTON_CLASS}
                title={plexSubmitting ? t("auth.plexPinFlowPending") : undefined}
              >
                {plexSubmitting ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <img
                    src="/auth-providers/plex.svg"
                    alt=""
                    aria-hidden="true"
                    className="h-4 w-4"
                  />
                )}
                {plexSubmitting ? t("auth.plexPinFlowPending") : t("auth.signInWithPlex")}
              </button>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

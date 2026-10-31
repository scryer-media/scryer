import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Navigate, useLocation, useNavigate } from "react-router";
import { useQuery, useMutation } from "urql";
import { authRuntimeStateQuery } from "@/lib/graphql/queries";
import { completeInitialAuthenticationMutation } from "@/lib/graphql/mutations";
import {
  AUTH_SESSION_CHANGED_EVENT,
  refreshInitialAuthenticationSession,
  useAuth,
  type AuthUser,
} from "@/lib/hooks/use-auth";
import type { AuthRuntimeState } from "@/lib/types/settings";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import {
  initialAuthenticationView,
  initialAuthenticationReloadPath,
  canCompleteInitialAuthentication,
} from "@/lib/utils/initial-authentication";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SetupIntroLoading, useSetupIntroTarget } from "./setup-intro";
import { SetupRestoreView } from "./setup-restore-view";

export function InitialAuthenticationGate({
  children,
}: {
  children: ReactNode;
}) {
  const t = useTranslate();
  const location = useLocation();
  const [{ data, error, fetching }, refresh] = useQuery<{
    authRuntimeState: AuthRuntimeState;
  }>({
    query: authRuntimeStateQuery,
    requestPolicy: "network-only",
  });
  const stage = initialAuthenticationView(
    data?.authRuntimeState?.initialAuthenticationStage,
  );
  const [observedStage, setObservedStage] =
    useState<ReturnType<typeof initialAuthenticationView>>("error");
  const navigate = useNavigate();
  const [handoff, setHandoff] = useState<AuthRuntimeState | null>(null);
  const reloadPath = initialAuthenticationReloadPath(
    observedStage,
    stage,
    handoff !== null,
  );
  const completeHandoff = useCallback(
    async (adopt: () => void) => {
      setHandoff(data!.authRuntimeState);
      try {
        await Promise.all([
          refreshInitialAuthenticationSession(adopt),
          import("@/src/pages/setup"),
        ]);
        setObservedStage("application");
        navigate("/setup", { replace: true });
      } finally {
        setHandoff(null);
        refresh({ requestPolicy: "network-only" });
      }
    },
    [data, navigate, refresh],
  );
  const authoritative = !fetching && !error && data && stage !== "error";
  if (authoritative && observedStage !== stage && !reloadPath) {
    setObservedStage(stage);
  }
  useEffect(() => {
    if (!authoritative || !reloadPath) return;
    const base = getRuntimeBasePath();
    window.location.replace(`${base === "/" ? "" : base}${reloadPath}`);
  }, [authoritative, reloadPath]);
  const pending = stage === "choice" || stage === "password_replacement";
  useSetupIntroTarget(
    authoritative &&
      !handoff &&
      stage !== "choice" &&
      location.pathname !== "/setup",
  );
  useEffect(() => {
    const recheck = () => refresh({ requestPolicy: "network-only" });
    window.addEventListener(AUTH_SESSION_CHANGED_EVENT, recheck);
    window.addEventListener("focus", recheck);
    return () => {
      window.removeEventListener(AUTH_SESSION_CHANGED_EVENT, recheck);
      window.removeEventListener("focus", recheck);
    };
  }, [refresh]);
  useEffect(() => {
    if (pending) refresh({ requestPolicy: "network-only" });
  }, [location.key, pending, refresh]);
  if (fetching && (!data || observedStage === "error"))
    return <SetupIntroLoading />;
  if (error || !data || stage === "error")
    return (
      <SetupFrame>
        <h1 className="text-2xl font-semibold">{t("initialAuth.title")}</h1>
        <p role="alert">{t("initialAuth.loadFailed")}</p>
        <Button onClick={() => refresh({ requestPolicy: "network-only" })}>
          {t("initialAuth.retry")}
        </Button>
      </SetupFrame>
    );
  if (reloadPath) return <SetupIntroLoading />;
  if (stage === "choice" || handoff)
    return (
      <AuthenticationChoice
        state={handoff ?? data.authRuntimeState}
        onStale={() => refresh({ requestPolicy: "network-only" })}
        onComplete={completeHandoff}
      />
    );
  if (stage === "password_replacement" && location.pathname !== "/login")
    return <Navigate to="/login?redirect=%2Fsetup" replace />;
  return children;
}

function SetupFrame({
  children,
  panel = true,
}: {
  children: ReactNode;
  panel?: boolean;
}) {
  const logoRef = useSetupIntroTarget<HTMLImageElement>();
  return (
    <main className="min-h-screen bg-fixed px-6 py-16 text-foreground [background-image:var(--scry-shell-bg)]">
      <img
        ref={logoRef}
        src={`${import.meta.env.BASE_URL}scryer-logo.svg`}
        alt=""
        aria-hidden="true"
        width={80}
        height={80}
        className="mx-auto mb-8 h-20 w-20"
      />
      <div
        className={
          panel
            ? "mx-auto flex max-w-lg flex-col gap-6 rounded-xl border bg-card p-8"
            : "mx-auto w-full max-w-2xl"
        }
      >
        {children}
      </div>
    </main>
  );
}

function SetupCodeInput({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const inputs = useRef<Array<HTMLInputElement | null>>([]);
  const characters = Array.from(
    { length: 6 },
    (_, index) => value[index]?.trim() ?? "",
  );
  const enter = (index: number, text: string) => {
    const incoming = text.toUpperCase().replace(/[^A-Z0-9]/g, "");
    if (incoming.length >= 6) index = 0;
    const next = [...characters];
    if (!incoming) next[index] = "";
    else
      for (
        let offset = 0;
        offset < incoming.length && index + offset < 6;
        offset++
      ) {
        next[index + offset] = incoming[offset];
      }
    onChange(
      next
        .map((character) => character || " ")
        .join("")
        .trimEnd(),
    );
    if (incoming) inputs.current[Math.min(index + incoming.length, 5)]?.focus();
  };
  return (
    <fieldset
      id="initial-authentication-setup-code"
      aria-describedby="setup-code-help"
    >
      <legend className="mb-2">{label}</legend>
      <div className="flex items-center gap-1.5 sm:gap-2">
        {characters.map((character, index) => (
          <div key={index} className="contents">
            {index === 3 && (
              <span aria-hidden="true" className="font-mono text-2xl">
                -
              </span>
            )}
            <Input
              ref={(element) => {
                inputs.current[index] = element;
              }}
              id={`initial-authentication-setup-code-${index + 1}`}
              aria-label={`${label} ${index + 1}`}
              autoComplete={index === 0 ? "one-time-code" : "off"}
              autoCapitalize="characters"
              spellCheck={false}
              value={character}
              required
              className="h-14 min-w-0 flex-1 px-0 text-center font-mono text-2xl md:text-2xl"
              onFocus={(event) => event.currentTarget.select()}
              onChange={(event) => enter(index, event.target.value)}
              onPaste={(event) => {
                event.preventDefault();
                enter(index, event.clipboardData.getData("text"));
              }}
              onKeyDown={(event) => {
                if (
                  event.key === "ArrowLeft" ||
                  (event.key === "Backspace" && !character)
                ) {
                  event.preventDefault();
                  inputs.current[Math.max(0, index - 1)]?.focus();
                } else if (event.key === "ArrowRight") {
                  event.preventDefault();
                  inputs.current[Math.min(5, index + 1)]?.focus();
                }
              }}
            />
          </div>
        ))}
      </div>
    </fieldset>
  );
}

function AuthenticationChoice({
  state,
  onStale,
  onComplete,
}: {
  state: AuthRuntimeState;
  onStale: () => void;
  onComplete: (adopt: () => void) => Promise<void>;
}) {
  const t = useTranslate();
  const { adoptSession } = useAuth();
  const [mode, setMode] = useState<"PASSWORD" | "NO_AUTHENTICATION" | null>(
    null,
  );
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [setupCode, setSetupCode] = useState("");
  const [restoring, setRestoring] = useState(false);
  const [acknowledged, setAcknowledged] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [transitioning, setTransitioning] = useState(false);
  const [{ fetching: mutationFetching }, complete] = useMutation<{
    completeInitialAuthentication: {
      completed: boolean;
      login: {
        token: string;
        persistSession: boolean;
        user: AuthUser | null;
      } | null;
    };
  }>(completeInitialAuthenticationMutation);
  const fetching = mutationFetching || transitioning;
  const minimum = state.passwordMinLength ?? 8;
  const blockedByEnvironment = (choice: string) =>
    state.envOverrideActive &&
    (choice === "PASSWORD") !== state.effectiveFormLoginEnabled;
  const canSubmit = canCompleteInitialAuthentication({
    mode,
    password,
    confirmation,
    minimum,
    acknowledged,
    codeRequired: state.initialAuthenticationCodeRequired === true,
    code: setupCode,
    environmentMode: state.envOverrideActive
      ? state.effectiveFormLoginEnabled
      : null,
  });
  const submit = useCallback(
    async (event: React.FormEvent) => {
      event.preventDefault();
      if (!canSubmit || fetching) return;
      setError(null);
      const result = await complete({
        input: {
          mode,
          password: mode === "PASSWORD" ? password : null,
          acknowledgeNoAuthentication:
            mode === "NO_AUTHENTICATION" && acknowledged,
          setupCode: state.initialAuthenticationCodeRequired ? setupCode : null,
        },
      });
      if (
        result.error ||
        !result.data?.completeInitialAuthentication.completed
      ) {
        setError(
          userFacingGraphQlErrorMessage(
            result.error,
            t("initialAuth.saveFailed"),
            t,
          ),
        );
        onStale();
        return;
      }
      const login = result.data.completeInitialAuthentication.login;
      setTransitioning(true);
      try {
        await onComplete(() => {
          if (login)
            adoptSession(login.token, login.user, login.persistSession);
        });
      } catch (error) {
        setError(
          userFacingGraphQlErrorMessage(error, t("initialAuth.loadFailed"), t),
        );
        onStale();
      } finally {
        setTransitioning(false);
      }
    },
    [
      canSubmit,
      fetching,
      complete,
      mode,
      password,
      acknowledged,
      state.initialAuthenticationCodeRequired,
      setupCode,
      t,
      onStale,
      onComplete,
      adoptSession,
    ],
  );
  if (restoring) {
    return (
      <SetupFrame panel={false}>
        <SetupRestoreView
          t={t}
          setupCode={state.initialAuthenticationCodeRequired ? setupCode : null}
          onBack={() => setRestoring(false)}
        />
      </SetupFrame>
    );
  }
  return (
    <SetupFrame>
      <h1
        id="initial-authentication-heading"
        className="text-2xl font-semibold"
      >
        {t("initialAuth.title")}
      </h1>
      <p className="text-muted-foreground">{t("initialAuth.description")}</p>
      <form
        id="initial-authentication-form"
        onSubmit={(event) => void submit(event)}
        className="space-y-6"
      >
        <fieldset disabled={fetching} className="space-y-3">
          <legend className="mb-3 font-medium">
            {t("initialAuth.choose")}
          </legend>
          {(["PASSWORD", "NO_AUTHENTICATION"] as const).map((choice) => (
            <label key={choice} className="flex gap-3 rounded-lg border p-4">
              <input
                type="radio"
                id={
                  choice === "PASSWORD"
                    ? "initial-authentication-mode-password"
                    : "initial-authentication-mode-no-authentication"
                }
                name="authentication-mode"
                value={choice}
                checked={mode === choice}
                disabled={blockedByEnvironment(choice)}
                onChange={() => {
                  setMode(choice);
                  setError(null);
                }}
              />
              <span>
                {t(
                  choice === "PASSWORD"
                    ? "initialAuth.passwordMode"
                    : "initialAuth.noAuthMode",
                )}
              </span>
            </label>
          ))}
        </fieldset>
        {state.envOverrideActive && <p>{t("initialAuth.environment")}</p>}
        {mode === "PASSWORD" && (
          <div className="space-y-3">
            <p>{t("initialAuth.admin", { minLength: minimum })}</p>
            <label className="block">
              {t("initialAuth.password")}
              <Input
                type="password"
                autoComplete="new-password"
                required
                id="initial-authentication-password"
                minLength={minimum}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </label>
            <label className="block">
              {t("initialAuth.confirm")}
              <Input
                type="password"
                autoComplete="new-password"
                required
                id="initial-authentication-password-confirmation"
                value={confirmation}
                onChange={(e) => setConfirmation(e.target.value)}
              />
            </label>
            {confirmation && password !== confirmation && (
              <p role="alert">{t("initialAuth.mismatch")}</p>
            )}
          </div>
        )}
        {mode === "NO_AUTHENTICATION" && (
          <label className="flex gap-3">
            <input
              type="checkbox"
              id="initial-authentication-acknowledge"
              checked={acknowledged}
              onChange={(e) => setAcknowledged(e.target.checked)}
            />
            <span>{t("initialAuth.acknowledge")}</span>
          </label>
        )}
        {state.initialAuthenticationCodeRequired && (
          <div>
            <SetupCodeInput
              label={t("initialAuth.code")}
              value={setupCode}
              onChange={setSetupCode}
            />
            <span
              id="setup-code-help"
              className="mt-2 block text-sm text-muted-foreground"
            >
              {t("initialAuth.codeHelp")}
            </span>
          </div>
        )}
        {error && (
          <p id="initial-authentication-error" role="alert">
            {error}
          </p>
        )}
        <Button
          id="initial-authentication-submit"
          type="submit"
          disabled={!canSubmit || fetching}
          className="w-full"
        >
          {t(
            fetching
              ? "initialAuth.saving"
              : mode === "NO_AUTHENTICATION"
                ? "initialAuth.continueWithout"
                : "initialAuth.continue",
          )}
        </Button>
      </form>
      <Button
        type="button"
        variant="outline"
        disabled={
          fetching ||
          (state.initialAuthenticationCodeRequired &&
            setupCode.replace(/\s/g, "").length !== 6)
        }
        id="initial-authentication-restore"
        onClick={() => setRestoring(true)}
      >
        {t("setup.restoreTitle")}
      </Button>
    </SetupFrame>
  );
}

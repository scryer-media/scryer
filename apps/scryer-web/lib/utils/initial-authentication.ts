/** Only a recognized completed/legacy state may expose the application. */
export function initialAuthenticationView(stage: unknown) {
  if (stage === null || stage === "complete") return "application";
  if (stage === "choice" || stage === "password_replacement") return stage;
  return "error";
}

/** Rebootstrap the session before exposing a newly selected setup path. */
export function initialAuthenticationReloadPath(
  previous: ReturnType<typeof initialAuthenticationView>,
  next: ReturnType<typeof initialAuthenticationView>,
  refreshingLocalSession = false,
) {
  if (refreshingLocalSession) return null;
  if (previous === "error" || next === "error" || previous === next)
    return null;
  return next === "password_replacement"
    ? "/login?redirect=%2Fsetup"
    : "/setup";
}

export function canCompleteInitialAuthentication(input: {
  mode: "PASSWORD" | "NO_AUTHENTICATION" | null;
  password: string;
  confirmation: string;
  minimum: number;
  acknowledged: boolean;
  codeRequired: boolean;
  code: string;
  environmentMode: boolean | null;
}) {
  if (input.mode === null) return false;
  const passwordMode = input.mode === "PASSWORD";
  if (input.environmentMode !== null && input.environmentMode !== passwordMode)
    return false;
  if (input.codeRequired && input.code.replace(/[\s-]/g, "").length !== 6)
    return false;
  return passwordMode
    ? input.password.length >= input.minimum &&
        input.password === input.confirmation
    : input.acknowledged;
}

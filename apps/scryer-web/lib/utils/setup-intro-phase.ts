/**
 * `waiting` shows no logo: most page loads are not setup, and only the
 * authentication decision can say whether this one is.
 */
export type SetupIntroPhase = "waiting" | "playing" | "done";

export function initialSetupIntroPhase(
  pathname: string,
  flies: boolean,
): SetupIntroPhase {
  if (!flies) return "done";
  return pathname === "/setup" ? "playing" : "waiting";
}

/** A landing target means a setup screen opened; none means an ordinary page. */
export function setupIntroPhaseForTarget(
  phase: SetupIntroPhase,
  hasTarget: boolean,
): SetupIntroPhase {
  if (phase !== "waiting") return phase;
  return hasTarget ? "playing" : "done";
}

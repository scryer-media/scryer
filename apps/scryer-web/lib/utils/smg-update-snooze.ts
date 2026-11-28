// Dismissing the Scryer update reminder only hides it for this long. It then
// comes back, for the same release, until the instance is upgraded.
export const SMG_SCRYER_UPDATE_SNOOZE_MS = 24 * 60 * 60 * 1_000;

export type SmgScryerUpdateDismissal = { value: string; dismissedAt: number };

/**
 * Milliseconds the snooze still has to run at `now`, or 0 when it is over. A
 * dismissal dated after `now` (the clock was set back) is over too, so a bad
 * clock can never hide the reminder for longer than the snooze.
 */
export function smgScryerUpdateSnoozeRemainingMs(
  dismissal: SmgScryerUpdateDismissal,
  now: number,
): number {
  const elapsed = now - dismissal.dismissedAt;
  if (elapsed < 0 || elapsed >= SMG_SCRYER_UPDATE_SNOOZE_MS) {
    return 0;
  }
  return SMG_SCRYER_UPDATE_SNOOZE_MS - elapsed;
}

/**
 * The stored dismissal while its snooze still runs at `now`, else null. A
 * dismissal written before the reminder became a snooze is a bare string: it
 * carries no time, so it reads as expired and the reminder returns.
 */
export function parseSmgScryerUpdateDismissal(
  raw: string | null,
  now: number,
): SmgScryerUpdateDismissal | null {
  if (!raw) {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) {
    return null;
  }
  const { value, dismissedAt } = parsed as Partial<SmgScryerUpdateDismissal>;
  if (
    typeof value !== "string" ||
    typeof dismissedAt !== "number" ||
    !Number.isFinite(dismissedAt)
  ) {
    return null;
  }
  const dismissal = { value, dismissedAt };
  return smgScryerUpdateSnoozeRemainingMs(dismissal, now) > 0
    ? dismissal
    : null;
}

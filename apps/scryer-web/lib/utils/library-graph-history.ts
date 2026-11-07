const STATE_KEY = "libraryGraph";

type Entry<T> = { scope: string; snapshot: T };

/** Graph state lives in the router's user state, alongside other route state. */
export function graphHistoryState<T>(
  state: unknown,
  scope: string,
  snapshot: T,
) {
  return {
    ...(state && typeof state === "object" ? state : {}),
    [STATE_KEY]: { scope, snapshot },
  };
}

export function graphHistorySnapshot<T>(
  state: unknown,
  scope: string,
): T | null {
  if (!state || typeof state !== "object" || !(STATE_KEY in state)) return null;
  const entry = (state as Record<string, unknown>)[
    STATE_KEY
  ] as Entry<T> | null;
  return entry?.scope === scope ? entry.snapshot : null;
}

/** Prevent a pending router transition from overwriting the entry it leaves. */
export function createGraphHistorySync<T>() {
  let previousEntry: string | null = null;
  let pendingPush = false;
  return {
    beginPush() {
      pendingPush = true;
    },
    visit(
      key: string,
      navigation: string,
      state: unknown,
      scope: string,
    ):
      | { kind: "skip" }
      | { kind: "persist" }
      | { kind: "restore"; snapshot: T } {
      const changed = previousEntry !== key;
      if (pendingPush && !changed) return { kind: "skip" };
      pendingPush = false;
      previousEntry = key;
      const saved =
        changed && navigation === "POP"
          ? graphHistorySnapshot<T>(state, scope)
          : null;
      return saved === null
        ? { kind: "persist" }
        : { kind: "restore", snapshot: saved };
    },
  };
}

export type PendingReauthenticationAction<Action> = {
  action: Action;
  queuedAtGeneration: number;
};

export function queueReauthenticationAction<Action>(
  action: Action,
  generation: number,
): PendingReauthenticationAction<Action> {
  return { action, queuedAtGeneration: generation };
}

export function consumeReauthenticatedAction<Action>(
  pending: PendingReauthenticationAction<Action> | null,
  generation: number,
): { pending: PendingReauthenticationAction<Action> | null; action: Action | null } {
  if (!pending || generation <= pending.queuedAtGeneration) {
    return { pending, action: null };
  }

  return { pending: null, action: pending.action };
}

export function dismissReauthenticationAction(): null {
  return null;
}

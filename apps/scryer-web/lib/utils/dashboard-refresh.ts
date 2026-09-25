export type DashboardPanelKey =
  "overview" | "storage" | "requests" | "imports" | "recent" | "queue";
export type DashboardPanelState = {
  loading: boolean;
  ready: boolean;
  error: string | null;
};
export type DashboardPanelStates = Record<
  DashboardPanelKey,
  DashboardPanelState
>;

/** A panel fed by several loads: ready only when all are, loading while any is, first error wins. */
export function combinePanelStates(...states: DashboardPanelState[]): DashboardPanelState {
  return {
    loading: states.some((state) => state.loading),
    ready: states.every((state) => state.ready),
    error: states.find((state) => state.error)?.error ?? null,
  };
}

export function initialDashboardPanelStates(): DashboardPanelStates {
  return Object.fromEntries(
    ["overview", "storage", "requests", "imports", "recent", "queue"].map(
      (key) => [key, { loading: true, ready: false, error: null }],
    ),
  ) as DashboardPanelStates;
}

/** Coalesce a panel's work and publish only while its owning view is live. */
export function createDashboardRefresh(
  update: (key: DashboardPanelKey, patch: Partial<DashboardPanelState>) => void,
) {
  let generation = 0;
  let active = true;
  const queued = new Map<DashboardPanelKey, () => Promise<() => void>>();
  const pending = new Map<DashboardPanelKey, Promise<void>>();
  return {
    activate() {
      active = true;
    },
    dispose() {
      active = false;
      generation += 1;
      pending.clear();
      queued.clear();
    },
    run(
      key: DashboardPanelKey,
      load: () => Promise<() => void>,
      invalidate = false,
    ): Promise<void> {
      if (!active) return Promise.resolve();
      const existing = pending.get(key);
      if (existing) {
        if (invalidate) queued.set(key, load);
        return existing;
      }
      const started = generation;
      update(key, { loading: true, error: null });
      const promise = Promise.resolve()
        .then(async () => {
          let next: (() => Promise<() => void>) | undefined = load;
          while (next && active && generation === started) {
            try {
              const publish = await next();
              if (active && generation === started && !queued.has(key)) {
                publish();
                update(key, { ready: true, error: null });
              }
            } catch (error: unknown) {
              if (active && generation === started && !queued.has(key)) {
                update(key, {
                  error: error instanceof Error ? error.message : String(error),
                });
              }
            }
            if (!active || generation !== started) return;
            next = queued.get(key);
            queued.delete(key);
          }
        })
        .finally(() => {
          if (active && generation === started) {
            pending.delete(key);
            update(key, { loading: false });
          }
        });
      pending.set(key, promise);
      return promise;
    },
  };
}

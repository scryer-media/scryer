export const GRAPH_HOVER_DELAY_MS = 75;

/** A quick pass never activates a hover; replacement and teardown cancel it. */
export function createGraphHoverIntent() {
  let candidate: string | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const cancel = () => {
    candidate = null;
    clearTimeout(timer);
    timer = undefined;
  };
  return {
    enter(key: string, activate: () => void) {
      if (candidate === key) return;
      cancel();
      candidate = key;
      timer = setTimeout(() => {
        timer = undefined;
        if (candidate === key) activate();
      }, GRAPH_HOVER_DELAY_MS);
    },
    leave(key: string) {
      if (candidate === key) cancel();
    },
    cancel,
  };
}

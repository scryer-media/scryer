import * as React from "react";

import {
  createBackoffPoller,
  type BackoffPoller,
} from "@/lib/utils/backoff-poller";

/**
 * A backed-off poll owned by a component: a poll held back while the tab was
 * hidden runs when it is shown again, and the poll stops when the component
 * unmounts. `poll` resolves true while there is still something to watch.
 */
export function useBackoffPoller(
  delaysMs: readonly number[],
  poll: () => Promise<boolean>,
): BackoffPoller {
  // The poller lives as long as the component; each poll calls the latest
  // `poll` the component rendered with.
  const [{ latest, poller }] = React.useState(() => {
    const latest = { poll };
    return {
      latest,
      poller: createBackoffPoller({ delaysMs, poll: () => latest.poll() }),
    };
  });
  React.useEffect(() => {
    latest.poll = poll;
  }, [latest, poll]);

  React.useEffect(() => {
    if (typeof document === "undefined") {
      return () => poller.stop();
    }
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") poller.resume();
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      document.removeEventListener("visibilitychange", onVisibilityChange);
      poller.stop();
    };
  }, [poller]);

  return poller;
}

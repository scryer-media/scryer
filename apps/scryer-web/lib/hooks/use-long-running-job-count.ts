import * as React from "react";

import { useJobRunToasts } from "@/components/root/job-run-provider";
import { longRunningJobs } from "@/lib/utils/job-runs";

/**
 * How many jobs have been running long enough to call out in navigation, one
 * per job. Re-evaluates when the next active job crosses the threshold, so a
 * job joins the count without waiting for its next progress event.
 */
export function useLongRunningJobCount(): number {
  const { runs } = useJobRunToasts();
  const [nowMs, setNowMs] = React.useState(() => Date.now());
  const { count, nextChangeAtMs } = React.useMemo(
    () => longRunningJobs(runs, nowMs),
    [runs, nowMs],
  );

  React.useEffect(() => {
    if (nextChangeAtMs === null) return;
    const timer = setTimeout(
      () => setNowMs(Date.now()),
      Math.max(0, nextChangeAtMs - Date.now()),
    );
    return () => clearTimeout(timer);
  }, [nextChangeAtMs]);

  // A run that arrives already past the threshold counts on arrival.
  React.useEffect(() => {
    setNowMs(Date.now());
  }, [runs]);

  return count;
}

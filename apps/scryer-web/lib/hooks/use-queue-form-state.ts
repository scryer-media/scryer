import { useState } from "react";

export function useQueueFormState() {
  const [titleNameForQueue, setTitleNameForQueue] = useState("");
  const [monitoredForQueue, setMonitoredForQueue] = useState(true);
  const [seasonFoldersForQueue, setSeasonFoldersForQueue] = useState(true);
  const [minAvailabilityForQueue, setMinAvailabilityForQueue] = useState("announced");

  return {
    titleNameForQueue,
    setTitleNameForQueue,
    monitoredForQueue,
    setMonitoredForQueue,
    seasonFoldersForQueue,
    setSeasonFoldersForQueue,
    minAvailabilityForQueue,
    setMinAvailabilityForQueue,
  };
}

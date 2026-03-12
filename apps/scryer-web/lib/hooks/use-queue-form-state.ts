import { useState } from "react";

export function useQueueFormState() {
  const [titleNameForQueue, setTitleNameForQueue] = useState("");
  const [monitoredForQueue, setMonitoredForQueue] = useState(true);
  const [seasonFoldersForQueue, setSeasonFoldersForQueue] = useState(true);
  const [monitorSpecialsForQueue, setMonitorSpecialsForQueue] = useState(false);
  const [interSeasonMoviesForQueue, setInterSeasonMoviesForQueue] = useState(true);
  const [minAvailabilityForQueue, setMinAvailabilityForQueue] = useState("announced");

  return {
    titleNameForQueue,
    setTitleNameForQueue,
    monitoredForQueue,
    setMonitoredForQueue,
    seasonFoldersForQueue,
    setSeasonFoldersForQueue,
    monitorSpecialsForQueue,
    setMonitorSpecialsForQueue,
    interSeasonMoviesForQueue,
    setInterSeasonMoviesForQueue,
    minAvailabilityForQueue,
    setMinAvailabilityForQueue,
  };
}

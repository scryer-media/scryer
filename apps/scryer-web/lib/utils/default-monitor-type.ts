export function resolveDefaultMonitorType(facet: string, saved: string | null | undefined, fallback: string): string {
  const allowed = facet === "MOVIE" ? ["MONITORED", "UNMONITORED"] :
    ["FUTURE_EPISODES", "MISSING_AND_FUTURE_EPISODES", "ALL_EPISODES", "NONE"];
  return saved && allowed.includes(saved) ? saved : fallback;
}

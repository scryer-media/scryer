// No imports, so this module stays loadable by plain `node --test`.

/** The history row fields the hover card reads. */
export type HistoryHoverEvent = {
  facet: string | null;
  posterUrl?: string | null;
  dataJson: unknown;
};

export type HistoryHoverFacet = "MOVIE" | "SERIES" | "ANIME";

export type HistoryHoverDetails = {
  facet: HistoryHoverFacet | null;
  posterUrl: string | null;
};

/** Reads an event payload (an object or its JSON text) as a plain record. */
export function historyHoverData(
  value: unknown,
): Record<string, unknown> | null {
  if (typeof value === "string") {
    try {
      return historyHoverData(JSON.parse(value));
    } catch {
      return null;
    }
  }
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return value as Record<string, unknown>;
  }
  return null;
}

/** A non-blank string field of a payload record, or null. */
export function historyHoverString(
  value: Record<string, unknown> | null,
  key: string,
): string | null {
  const candidate = value?.[key];
  return typeof candidate === "string" && candidate.trim() ? candidate : null;
}

/**
 * The title snapshot an event payload carries. A domain event serializes as
 * `{ type, data: { title, ... } }`; rematch and move payloads are serialized
 * bare, so their `title` (when present) sits at the top level.
 */
function historyPayloadTitle(
  payload: Record<string, unknown> | null,
): Record<string, unknown> | null {
  const data = historyHoverData(payload?.data);
  return (
    historyHoverData(data?.title) ?? historyHoverData(payload?.title) ?? null
  );
}

/**
 * What the history hover card shows for an event: the title's facet and its
 * poster. The poster comes from the live title row the API hydrates onto the
 * event first, then from the title snapshot recorded in the event payload.
 */
export function historyHoverDetails(
  event: HistoryHoverEvent,
): HistoryHoverDetails {
  const payload = historyHoverData(event.dataJson);
  const title = historyPayloadTitle(payload);
  const rawFacet = historyHoverString(title, "facet") ?? event.facet;
  const facet = rawFacet?.trim().toUpperCase() ?? null;
  const livePoster =
    typeof event.posterUrl === "string" && event.posterUrl.trim()
      ? event.posterUrl
      : null;
  return {
    facet:
      facet === "MOVIE" || facet === "SERIES" || facet === "ANIME"
        ? facet
        : null,
    posterUrl:
      livePoster ??
      historyHoverString(title, "poster_url") ??
      historyHoverString(title, "posterUrl"),
  };
}

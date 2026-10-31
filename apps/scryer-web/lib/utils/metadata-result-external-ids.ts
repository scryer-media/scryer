import type { Facet } from "@/lib/types/titles";

/** An external id as `addTitle` and the request inputs take it. */
export type KindedExternalIdInput = {
  source: string;
  kind?: string;
  value: string;
};

/** The identity fields a metadata search result carries. */
export type MetadataResultIdentity = {
  tvdbId: string;
  smgId?: number | null;
  tmdbId?: number | null;
  imdbId: string | null;
  externalIds?: Array<{ source: string; kind?: string | null; value: string }>;
};

/**
 * Whether a TMDB id's kind names a series: unkinded ids were always read as
 * the title's own, and `series` says so explicitly. Anything else (`movie`,
 * for an anime title's mapped films) is another entity's id.
 */
export function tmdbKindNamesSeries(kind: string | null | undefined): boolean {
  const normalized = kind?.trim().toLowerCase() ?? "";
  return normalized === "" || normalized === "series";
}

/**
 * The kind an id names at its source when the source did not say. An SMG id
 * names an SMG title; a TMDB id names a movie or a series, and TMDB reuses the
 * same number for both, so a series' TMDB id must say it is a series. Other
 * ids stay kindless (matching a stored id of any kind), as they were before.
 */
function externalIdKind(source: string, facet: Facet): string | undefined {
  if (source === "smg") return "title";
  if (source === "tmdb" && facet !== "MOVIE") return "series";
  return undefined;
}

/**
 * Every external id a metadata search result names, deduplicated and kinded,
 * for adding or requesting it. A TMDB-primary series has no TVDB id: its SMG
 * title id and its TMDB series id identify it.
 *
 * An id the result already kinds keeps that kind: an anime's `tmdb:movie`
 * id must not be re-labelled as the series' own. Only the ids synthesized
 * from the result's `smgId` / `tmdbId` fields take the facet-derived kind.
 */
export function metadataResultExternalIds(
  result: MetadataResultIdentity,
  facet: Facet,
): KindedExternalIdInput[] {
  const smgId = result.smgId == null ? "" : String(result.smgId).trim();
  const tvdbId = String(result.tvdbId ?? "").trim();
  const tmdbId = result.tmdbId == null ? "" : String(result.tmdbId).trim();
  const imdbId = result.imdbId?.trim();
  const seen = new Set<string>();
  const ids: KindedExternalIdInput[] = [];
  const candidates: Array<{
    source: string;
    kind?: string | null;
    value: string;
  }> = [
    ...(result.externalIds ?? []),
    ...(smgId ? [{ source: "smg", value: smgId }] : []),
    ...(tvdbId ? [{ source: "tvdb", value: tvdbId }] : []),
    ...(tmdbId ? [{ source: "tmdb", value: tmdbId }] : []),
    ...(imdbId ? [{ source: "imdb", value: imdbId }] : []),
  ];
  for (const externalId of candidates) {
    const source = externalId.source.trim().toLowerCase();
    const value = externalId.value.trim();
    const key = `${source}:${value}`;
    if (!source || !value || seen.has(key)) {
      continue;
    }
    seen.add(key);
    const suppliedKind = externalId.kind?.trim().toLowerCase();
    const kind = suppliedKind || externalIdKind(source, facet);
    ids.push(kind ? { source, kind, value } : { source, value });
  }
  return ids;
}

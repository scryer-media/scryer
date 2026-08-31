import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { CatalogDiscoveryItem, ExternalId, Facet } from "@/lib/types";
import { discoveryItemDisplayTitle } from "@/lib/utils/discovery-display";

export function normalizedDiscoveryItemFacet(
  value: string | null | undefined,
): Facet | null {
  switch (value?.trim().toLowerCase()) {
    case "anime":
      return "ANIME";
    case "series":
      return "SERIES";
    case "movie":
      return "MOVIE";
    default:
      return null;
  }
}

export function discoveryItemFacet(
  item: Pick<CatalogDiscoveryItem, "contentType" | "targetKind">,
): Facet | null {
  const contentType = item.contentType?.trim();
  return contentType
    ? normalizedDiscoveryItemFacet(contentType)
    : normalizedDiscoveryItemFacet(item.targetKind);
}

type DiscoveryExternalIdSignals = {
  targetKey: string;
  externalIds?: Array<{
    source?: string | null;
    kind?: string | null;
    id?: string | null;
    key?: string | null;
  }> | null;
  sourceTags?: string[] | null;
};

export type DiscoveryResolvedExternalId = ExternalId & {
  kind: string | null;
  key: string | null;
};

const EXTERNAL_ID_SOURCE_ALIASES: Record<string, string> = {
  anidb: "anidb",
  anidbnet: "anidb",
  anilist: "anilist",
  anilistco: "anilist",
  imdb: "imdb",
  imdbcom: "imdb",
  mal: "mal",
  myanimelist: "mal",
  myanimelistnet: "mal",
  themoviedb: "tmdb",
  themoviedborg: "tmdb",
  tmdb: "tmdb",
  trakt: "trakt",
  traktv: "trakt",
  thetvdb: "tvdb",
  thetvdbcom: "tvdb",
  tvdb: "tvdb",
};

function normalizedExternalIdSource(value: string | null | undefined) {
  const normalized = value?.trim().toLowerCase().replace(/[\s_.-]+/g, "");
  return normalized ? (EXTERNAL_ID_SOURCE_ALIASES[normalized] ?? null) : null;
}

function externalIdFromUrl(value: string): DiscoveryResolvedExternalId | null {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return null;
  }

  const host = url.hostname.toLowerCase().replace(/^www\./, "");
  const path = url.pathname;
  if (host === "imdb.com") {
    const match = path.match(/\/title\/(tt\d+)/i);
    return match
      ? { source: "imdb", value: match[1], kind: "movie", key: value }
      : null;
  }
  if (host === "themoviedb.org") {
    const match = path.match(/\/(movie|tv)\/(\d+)/i);
    return match
      ? {
          source: "tmdb",
          value: match[2],
          kind: match[1].toLowerCase(),
          key: value,
        }
      : null;
  }
  if (host === "thetvdb.com") {
    const match = path.match(
      /\/(?:dereferrer\/)?(movie|movies|series)\/(\d+)/i,
    );
    return match
      ? {
          source: "tvdb",
          value: match[2],
          kind: match[1].toLowerCase().startsWith("movie")
            ? "movie"
            : "series",
          key: value,
        }
      : null;
  }
  if (host === "myanimelist.net") {
    const match = path.match(/\/anime\/(\d+)/i);
    return match
      ? { source: "mal", value: match[1], kind: "anime", key: value }
      : null;
  }
  if (host === "anilist.co") {
    const match = path.match(/\/anime\/(\d+)/i);
    return match
      ? { source: "anilist", value: match[1], kind: "anime", key: value }
      : null;
  }
  if (host === "anidb.net") {
    const match = path.match(/\/anime\/(\d+)/i);
    return match
      ? { source: "anidb", value: match[1], kind: "anime", key: value }
      : null;
  }
  return null;
}

function externalIdFromDiscoveryKey(value: string): DiscoveryResolvedExternalId | null {
  const urlExternalId = externalIdFromUrl(value);
  if (urlExternalId) {
    return urlExternalId;
  }

  const parts = value
    .split(":")
    .map((part) => part.trim())
    .filter(Boolean);
  const source = normalizedExternalIdSource(parts[0]);
  if (!source) {
    return null;
  }
  const id =
    source === "imdb"
      ? (parts.find((part) => /^tt\d+$/i.test(part)) ?? parts.at(-1))
      : parts.at(-1);
  const kind = parts.length > 2 ? parts[1].toLowerCase() : null;
  return id ? { source, value: id, kind, key: value } : null;
}

function externalIdFromExplicitDiscoveryId(
  value: NonNullable<DiscoveryExternalIdSignals["externalIds"]>[number],
): DiscoveryResolvedExternalId | null {
  const source = normalizedExternalIdSource(value.source);
  const kind = value.kind?.trim().toLowerCase() || null;
  const id = value.id?.trim();
  if (source && id) {
    return { source, value: id, kind, key: value.key?.trim() || null };
  }

  const key = value.key?.trim();
  if (!key) {
    return null;
  }
  const keyExternalId = externalIdFromDiscoveryKey(key);
  if (keyExternalId && (!source || keyExternalId.source === source)) {
    return {
      ...keyExternalId,
      kind: kind ?? keyExternalId.kind,
    };
  }
  return source ? { source, value: key, kind, key } : null;
}

export function richExternalIdsFromDiscoverySignals(
  item: DiscoveryExternalIdSignals,
): DiscoveryResolvedExternalId[] {
  const ids = new Map<string, DiscoveryResolvedExternalId>();
  for (const explicitId of item.externalIds ?? []) {
    const externalId = externalIdFromExplicitDiscoveryId(explicitId);
    if (externalId) {
      const mapKey = `${externalId.source}:${externalId.kind ?? ""}:${externalId.value}`;
      if (!ids.has(mapKey)) {
        ids.set(mapKey, externalId);
      }
    }
  }
  if (ids.size > 0) {
    return Array.from(ids.values());
  }

  for (const candidate of [item.targetKey, ...(item.sourceTags ?? [])]) {
    const externalId = externalIdFromDiscoveryKey(candidate);
    if (externalId) {
      const mapKey = `${externalId.source}:${externalId.kind ?? ""}:${externalId.value}`;
      if (!ids.has(mapKey)) {
        ids.set(mapKey, externalId);
      }
    }
  }
  return Array.from(ids.values());
}

export function externalIdsFromDiscoverySignals(
  item: DiscoveryExternalIdSignals,
): ExternalId[] {
  const ids = new Map<string, string>();
  for (const externalId of richExternalIdsFromDiscoverySignals(item)) {
    if (!ids.has(externalId.source)) {
      ids.set(externalId.source, externalId.value);
    }
  }
  return Array.from(ids, ([source, value]) => ({ source, value }));
}

export function externalIdsForDiscoveryItem(
  item: CatalogDiscoveryItem,
): ExternalId[] {
  return externalIdsFromDiscoverySignals(item);
}

export function metadataResultForDiscoveryItem(
  item: CatalogDiscoveryItem,
): MetadataTvdbSearchItem {
  const externalIds = externalIdsForDiscoveryItem(item);
  return {
    smgId: Number(
      externalIds.find((externalId) => externalId.source === "smg")?.value,
    ) || null,
    tvdbId:
      externalIds.find((externalId) => externalId.source === "tvdb")?.value ??
      "",
    tmdbId: Number(
      externalIds.find((externalId) => externalId.source === "tmdb")?.value,
    ) || null,
    name: discoveryItemDisplayTitle(item),
    imdbId:
      externalIds.find((externalId) => externalId.source === "imdb")?.value ??
      null,
    externalIds,
    slug: null,
    type: item.contentType ?? item.targetKind,
    year: item.year,
    status: item.statusTags[0] ?? null,
    overview: item.overview ?? null,
    popularity: item.rankScore,
    posterUrl: item.posterUrl,
    backgroundUrl: item.backgroundUrl ?? null,
    language: null,
    runtimeMinutes: null,
    sortTitle: item.sortTitle,
    rating: item.rating ?? null,
    ratingSource: item.sources?.[0] ?? item.bestSource ?? null,
    externalRatings: item.externalRatings ?? [],
  };
}

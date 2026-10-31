export type LocalPosterVariant = "original" | "w250" | "w70";
export type LocalBackdropVariant = "original" | "w1280";
export type LocalEpisodeStillVariant = "original" | "w300";
export type MediaImageVariant =
  LocalPosterVariant | LocalBackdropVariant | LocalEpisodeStillVariant;

const PROXIED_MEDIA_IMAGE_PATH_RE = /^(.*\/images\/media\/[^/]+\/)([^/]+)$/;
const LOCAL_TITLE_POSTER_PATH_RE =
  /^(.*\/images\/titles\/[^/]+\/poster\/)(original|w500|w250|w70)$/;
const LOCAL_TITLE_BACKDROP_PATH_RE =
  /^(.*\/images\/titles\/[^/]+\/fanart\/)(original|w\d+)$/;

/** Selects a variant on Scryer's opaque media-image route. */
export function selectMediaImageVariantUrl(
  imageUrl: string | null | undefined,
  desiredVariant: MediaImageVariant,
): string | null | undefined {
  if (!imageUrl) {
    return imageUrl;
  }

  try {
    const parsed = new URL(imageUrl, "http://scryer.local");
    const match = parsed.pathname.match(PROXIED_MEDIA_IMAGE_PATH_RE);
    if (!match) {
      return imageUrl;
    }

    const [, prefix, currentVariant] = match;
    if (currentVariant === desiredVariant) {
      return imageUrl;
    }

    parsed.pathname = `${prefix}${desiredVariant}`;
    return isRelativeUrl(imageUrl)
      ? `${parsed.pathname}${parsed.search}${parsed.hash}`
      : parsed.toString();
  } catch {
    return imageUrl;
  }
}

export function selectPosterVariantUrl(
  posterUrl: string | null | undefined,
  desiredVariant: LocalPosterVariant,
): string | null | undefined {
  if (!posterUrl) {
    return posterUrl;
  }

  const proxiedUrl = selectMediaImageVariantUrl(posterUrl, desiredVariant);
  if (proxiedUrl !== posterUrl) {
    return proxiedUrl;
  }

  try {
    const parsed = new URL(posterUrl, "http://scryer.local");
    const match = parsed.pathname.match(LOCAL_TITLE_POSTER_PATH_RE);
    if (match) {
      const [, prefix, currentVariant] = match;
      if (currentVariant === "original" || currentVariant === desiredVariant) {
        return posterUrl;
      }

      parsed.pathname = `${prefix}${desiredVariant}`;
      parsed.searchParams.delete("v");
      return isRelativeUrl(posterUrl)
        ? `${parsed.pathname}${parsed.search}${parsed.hash}`
        : parsed.toString();
    }

    return posterUrl;
  } catch {
    return posterUrl;
  }
}

export function selectBackdropVariantUrl(
  backdropUrl: string | null | undefined,
  desiredVariant: LocalBackdropVariant,
): string | null | undefined {
  if (!backdropUrl) {
    return backdropUrl;
  }

  const proxiedUrl = selectMediaImageVariantUrl(backdropUrl, desiredVariant);
  if (proxiedUrl !== backdropUrl) {
    return proxiedUrl;
  }

  try {
    const parsed = new URL(backdropUrl, "http://scryer.local");
    const localMatch = parsed.pathname.match(LOCAL_TITLE_BACKDROP_PATH_RE);
    if (localMatch) {
      const [, prefix, currentVariant] = localMatch;
      if (currentVariant === desiredVariant) {
        return backdropUrl;
      }

      parsed.pathname = `${prefix}${desiredVariant}`;
      parsed.searchParams.delete("v");
      return isRelativeUrl(backdropUrl)
        ? `${parsed.pathname}${parsed.search}${parsed.hash}`
        : parsed.toString();
    }

    return backdropUrl;
  } catch {
    return backdropUrl;
  }
}

function isRelativeUrl(url: string): boolean {
  return !/^[a-zA-Z][a-zA-Z\d+\-.]*:/.test(url) && !url.startsWith("//");
}

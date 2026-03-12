export type LocalPosterVariant = "original" | "w500" | "w250" | "w70";

const LOCAL_TITLE_POSTER_PATH_RE = /^(.*\/images\/titles\/[^/]+\/poster\/)(original|w500|w250|w70)$/;

export function selectPosterVariantUrl(
  posterUrl: string | null | undefined,
  desiredVariant: LocalPosterVariant,
): string | null | undefined {
  if (!posterUrl) {
    return posterUrl;
  }

  try {
    const parsed = new URL(posterUrl, "http://scryer.local");
    const match = parsed.pathname.match(LOCAL_TITLE_POSTER_PATH_RE);
    if (!match) {
      return posterUrl;
    }

    const [, prefix, currentVariant] = match;
    if (currentVariant === "original" || currentVariant === desiredVariant) {
      return posterUrl;
    }

    parsed.pathname = `${prefix}${desiredVariant}`;
    return isRelativeUrl(posterUrl)
      ? `${parsed.pathname}${parsed.search}${parsed.hash}`
      : parsed.toString();
  } catch {
    return posterUrl;
  }
}

function isRelativeUrl(url: string): boolean {
  return !/^[a-zA-Z][a-zA-Z\d+\-.]*:/.test(url) && !url.startsWith("//");
}

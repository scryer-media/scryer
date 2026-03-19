import type { ComponentProps } from "react";

type TitlePosterProps = Omit<ComponentProps<"img">, "src"> & {
  /** Local AVIF URL (from posterUrl). */
  src?: string | null;
  /** Original source JPG URL (from posterSourceUrl) — used as <img> fallback. */
  sourceSrc?: string | null;
};

/**
 * Renders a title poster with AVIF-first, JPG-fallback via `<picture>`.
 *
 * When both `src` (local AVIF) and `sourceSrc` (original JPG) are available,
 * the browser picks AVIF if supported and falls back to the JPG otherwise.
 * When only one URL is available, it renders a plain `<img>`.
 */
export function TitlePoster({
  src,
  sourceSrc,
  alt,
  ...props
}: TitlePosterProps) {
  const avifUrl = src ?? undefined;
  const fallbackUrl = sourceSrc ?? avifUrl;

  if (!fallbackUrl) {
    return null;
  }

  if (avifUrl && sourceSrc) {
    return (
      <picture>
        <source srcSet={avifUrl} type="image/avif" />
        <img src={sourceSrc} alt={alt} {...props} />
      </picture>
    );
  }

  return <img src={fallbackUrl} alt={alt} {...props} />;
}

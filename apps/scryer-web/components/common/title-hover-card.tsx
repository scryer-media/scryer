import { type ReactNode, useState } from "react";
import { createPortal } from "react-dom";

import { artworkFallbackStyle } from "@/lib/utils/artwork-fallback";

export type TitleHoverCardPreview = {
  id: string;
  title: string;
  facet: string | null;
  posterUrl: string | null;
  anchor: {
    top: number;
    right: number;
    bottom: number;
    left: number;
  };
  badges?: ReactNode;
  subtitle?: ReactNode;
  summary?: ReactNode;
  details?: ReactNode;
  footer?: ReactNode;
  layout?: "movie" | "episode";
  width?: number;
  estimatedHeight?: number;
};

export function TitleHoverCard({
  preview,
  ariaLabel,
  onMouseEnter,
  onMouseLeave,
}: {
  preview: TitleHoverCardPreview;
  ariaLabel: string;
  onMouseEnter: () => void;
  onMouseLeave: () => void;
}) {
  const [failedPosterUrl, setFailedPosterUrl] = useState<string | null>(null);

  if (typeof document === "undefined") return null;

  const gap = 12;
  const viewportPadding = 12;
  const width = Math.min(preview.width ?? 360, window.innerWidth - viewportPadding * 2);
  const estimatedHeight = preview.estimatedHeight ?? 240;
  const fitsOnRight =
    preview.anchor.right + gap + width <= window.innerWidth - viewportPadding;
  const unclampedLeft = fitsOnRight
    ? preview.anchor.right + gap
    : preview.anchor.left - gap - width;
  const left = Math.max(
    viewportPadding,
    Math.min(unclampedLeft, window.innerWidth - width - viewportPadding),
  );
  const top = Math.max(
    viewportPadding,
    Math.min(
      preview.anchor.top +
        (preview.anchor.bottom - preview.anchor.top - estimatedHeight) / 2,
      window.innerHeight - estimatedHeight - viewportPadding,
    ),
  );
  const normalizedFacet = preview.facet?.toUpperCase();
  const fallbackTone =
    normalizedFacet === "MOVIE"
      ? "MOVIE"
      : normalizedFacet === "ANIME"
        ? "ANIME"
        : "SERIES";
  const layout = preview.layout ?? "movie";

  return createPortal(
    <aside
      role="dialog"
      aria-label={ariaLabel}
      className={`fc-scryer-hover-card is-${layout}`}
      style={{ left, top, width }}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
    >
      <div
        className="fc-scryer-hover-card-image-wrap"
        style={artworkFallbackStyle(preview.id, fallbackTone)}
      >
        {preview.posterUrl && failedPosterUrl !== preview.posterUrl ? (
          <img
            src={preview.posterUrl}
            alt=""
            className="fc-scryer-hover-card-image"
            onError={() => setFailedPosterUrl(preview.posterUrl)}
          />
        ) : null}
      </div>
      <div className="fc-scryer-hover-card-copy">
        {preview.badges ? (
          <div className="fc-scryer-hover-card-badges">{preview.badges}</div>
        ) : null}
        <h3 className="fc-scryer-hover-card-title">{preview.title}</h3>
        {preview.subtitle ? (
          <p className="fc-scryer-hover-card-episode-title">{preview.subtitle}</p>
        ) : null}
        {preview.summary ? (
          <p className="fc-scryer-hover-card-overview">{preview.summary}</p>
        ) : null}
        {preview.details}
        {preview.footer ? (
          <div className="fc-scryer-hover-card-footer">{preview.footer}</div>
        ) : null}
      </div>
    </aside>,
    document.body,
  );
}

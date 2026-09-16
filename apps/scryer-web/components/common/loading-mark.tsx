import { cn } from "@/lib/utils";

const LOADING_MARK_SRC = `${import.meta.env.BASE_URL}scryer-loading.webp`;
const LOADING_MARK_STILL_SRC = `${import.meta.env.BASE_URL}scryer-loading-still.webp`;

// Decode the mark as soon as the app loads and hold the image, so the overlay
// shown while the backend restarts can still draw it with no server to fetch
// it from.
const preloadedLoadingMark =
  typeof Image === "undefined" ? null : Object.assign(new Image(), { src: LOADING_MARK_SRC });
void preloadedLoadingMark;

/**
 * Scryer's loading indicator: the logo mark turning on its axis.
 *
 * It stands wherever a spinner would. Size it like the icon it replaces
 * (`h-4 w-4`, `size-3.5`); the artwork is square and scales to fit. The
 * artwork is white, so outside the dark theme it is darkened to read against a
 * light background. The mark is decoration: callers say "loading" in text, a
 * disabled control or a `role="status"` region, so it is hidden from assistive
 * technology unless `label` names it. Under reduced motion it holds still on
 * its first frame.
 *
 * `reveal` holds it back briefly before fading in, for a placeholder that most
 * loads replace before anyone would notice it.
 */
export function LoadingMark({
  className,
  label,
  reveal = false,
}: {
  className?: string;
  label?: string;
  reveal?: boolean;
}) {
  return (
    <picture className="contents">
      {/* The picture's box is dropped, so an unhidden source would sit in a
          flex row as an empty item and push the mark's neighbours a gap away. */}
      <source
        media="(prefers-reduced-motion: reduce)"
        srcSet={LOADING_MARK_STILL_SRC}
        className="hidden"
      />
      <img
        src={LOADING_MARK_SRC}
        width={163}
        height={160}
        alt={label ?? ""}
        aria-hidden={label ? undefined : true}
        draggable={false}
        className={cn(
          "inline-block size-4 flex-none select-none object-contain brightness-[0.3] dark:brightness-100",
          reveal && "animate-loading-reveal",
          className,
        )}
      />
    </picture>
  );
}

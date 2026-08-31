import { ArrowUpRight, CircleCheck, X } from "lucide-react";

import { toast } from "@/components/ui/sonner";
import { TitlePosterSlot } from "@/components/title-poster-slot";
import { Button } from "@/components/ui/button";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";

export type CatalogAddToastContent = {
  /** Catalog name as the backend stored it. */
  titleName: string;
  year?: number | null;
  posterUrl?: string | null;
  /** Headline above the title ("Added to catalog"). */
  headline: string;
  /** Optional second line, e.g. "Automatic search queued." */
  note?: string | null;
  /** Poster placeholder copy when the metadata result has no artwork. */
  posterEmptyLabel: string;
  viewLabel: string;
  dismissLabel: string;
  /**
   * Omitted by surfaces that have nowhere to navigate — the toast then shows
   * artwork and copy only.
   */
  onView?: () => void;
};

const CATALOG_ADD_TOAST_DURATION_MS = 10_000;

// The default toaster paints its own card chrome; this toast owns the frame so
// the poster can sit flush against the left edge.
const CATALOG_ADD_TOAST_CLASS_NAME =
  "!w-full !border-0 !bg-transparent !p-0 !shadow-none";

function CatalogAddToastCard({
  titleName,
  year,
  posterUrl,
  headline,
  note,
  posterEmptyLabel,
  viewLabel,
  dismissLabel,
  onView,
  onDismiss,
}: CatalogAddToastContent & { onDismiss: () => void }) {
  // Same variant the search results render, so the poster is already cached.
  const posterSrc = selectPosterVariantUrl(posterUrl, "w250");

  return (
    <div className="flex w-full items-stretch gap-3 overflow-hidden rounded-[12px] border border-[var(--scry-success-border)] bg-[var(--card)] shadow-[0_18px_48px_rgba(0,0,0,0.34)]">
      <div className="relative w-[68px] flex-none overflow-hidden bg-muted">
        <TitlePosterSlot
          src={posterSrc}
          alt={titleName}
          emptyLabel={posterEmptyLabel}
          className="absolute inset-0 h-full w-full object-cover"
          placeholderClassName="absolute inset-0 h-full w-full"
          fallbackTitle={titleName}
          // A thumbnail this narrow has no room for fallback copy, and the
          // hydration spinner would never resolve inside the toast's lifetime.
          fallbackShowText={false}
          loading="eager"
        />
      </div>
      <div className="flex min-w-0 flex-1 flex-col gap-2 py-3 pr-3">
        <div className="flex min-w-0 items-start gap-2">
          <CircleCheck
            aria-hidden="true"
            className="mt-0.5 size-4 flex-none text-[var(--scry-success-text)]"
          />
          <div className="min-w-0 flex-1">
            <p className="text-xs font-semibold uppercase tracking-wide text-[var(--scry-success-text)]">
              {headline}
            </p>
            <p className="truncate text-sm font-semibold text-[var(--scry-ink2)]">
              {titleName}
              {year ? (
                <span className="font-medium text-[var(--scry-muted2)]">
                  {" "}
                  ({year})
                </span>
              ) : null}
            </p>
            {note ? (
              <p className="truncate text-xs text-[var(--scry-muted2)]">
                {note}
              </p>
            ) : null}
          </div>
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            aria-label={dismissLabel}
            title={dismissLabel}
            onClick={onDismiss}
          >
            <X aria-hidden="true" />
          </Button>
        </div>
        {onView ? (
          <Button
            variant="primary"
            size="sm"
            className="self-start"
            onClick={() => {
              onDismiss();
              onView();
            }}
          >
            {viewLabel}
            <ArrowUpRight aria-hidden="true" />
          </Button>
        ) : null}
      </div>
    </div>
  );
}

/**
 * Success feedback for a catalog add. Surfaces that can navigate pass `onView`
 * so the user can jump to the new title without the add flow having to steal
 * the page from them.
 */
export function showCatalogAddToast(content: CatalogAddToastContent) {
  toast.custom(
    (toastId) => (
      <CatalogAddToastCard
        {...content}
        onDismiss={() => toast.dismiss(toastId)}
      />
    ),
    {
      className: CATALOG_ADD_TOAST_CLASS_NAME,
      duration: CATALOG_ADD_TOAST_DURATION_MS,
    },
  );
}

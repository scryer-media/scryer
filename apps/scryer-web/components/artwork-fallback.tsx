import * as React from "react";

import { cn } from "@/lib/utils";
import {
  artworkFallbackStyle,
  type ArtworkFallbackTone,
} from "@/lib/utils/artwork-fallback";
import { LoadingMark } from "@/components/common/loading-mark";

export function ArtworkFallback({
  className,
  ariaLabel,
  emptyLabel,
  showSpinner = false,
  title,
  subtitle,
  tone = "neutral",
  showText = true,
}: {
  className?: string;
  ariaLabel?: string;
  emptyLabel: string;
  showSpinner?: boolean;
  title?: string | null;
  subtitle?: string | number | null;
  tone?: ArtworkFallbackTone | null;
  showText?: boolean;
}) {
  const displayTitle = title?.trim() || emptyLabel;
  const displaySubtitle =
    subtitle != null && `${subtitle}`.trim() !== "" ? `${subtitle}` : null;
  const resolvedTone = tone ?? "neutral";
  const gradientStyle = React.useMemo(
    () => artworkFallbackStyle(displayTitle, resolvedTone),
    [displayTitle, resolvedTone],
  );

  return (
    <div
      className={cn("relative isolate overflow-hidden", className)}
      style={gradientStyle}
      aria-label={ariaLabel}
      role="img"
    >
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-[radial-gradient(circle_at_50%_18%,rgba(255,255,255,0.16),transparent_36%),linear-gradient(180deg,rgba(255,255,255,0.04),rgba(0,0,0,0.36))]"
      />
      {showSpinner ? (
        <div className="relative z-10 flex h-full w-full items-center justify-center">
          <LoadingMark className="h-5 w-5 text-white/60" />
        </div>
      ) : showText ? (
        <div className="relative z-10 flex h-full w-full flex-col items-center justify-end px-3 pb-5 text-center">
          <p className="line-clamp-3 font-[var(--font-space-grotesk)] text-base font-bold leading-tight text-white drop-shadow-[0_1px_3px_rgba(0,0,0,0.55)]">
            {displayTitle}
          </p>
          {displaySubtitle ? (
            <p className="mt-1 text-sm font-medium text-white/72">
              {displaySubtitle}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

import * as React from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";

import { cn } from "@/lib/utils";

type HorizontalRailProps = {
  children: React.ReactNode;
  className?: string;
  containerClassName?: string;
  fadeClassName?: string;
  scrollControlLabel?: string;
  scrollBackControlLabel?: string;
};

export function HorizontalRail({
  children,
  className,
  containerClassName,
  fadeClassName,
  scrollControlLabel = "Scroll right",
  scrollBackControlLabel = "Scroll left",
}: HorizontalRailProps) {
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const overflowRef = React.useRef({
    maxScrollLeft: 0,
    hasMoreLeft: false,
    hasMoreRight: false,
  });
  const [overflow, setOverflow] = React.useState({
    hasMoreLeft: false,
    hasMoreRight: false,
  });

  const updateControls = React.useCallback((scrollLeft: number) => {
    const current = overflowRef.current;
    const hasMoreLeft = scrollLeft > 1;
    const hasMoreRight = current.maxScrollLeft - scrollLeft > 1;
    if (
      current.hasMoreLeft === hasMoreLeft &&
      current.hasMoreRight === hasMoreRight
    ) {
      return;
    }

    overflowRef.current = { ...current, hasMoreLeft, hasMoreRight };
    setOverflow({ hasMoreLeft, hasMoreRight });
  }, []);

  const measureOverflow = React.useCallback(() => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }

    overflowRef.current.maxScrollLeft = Math.max(
      element.scrollWidth - element.clientWidth,
      0,
    );
    updateControls(element.scrollLeft);
  }, [updateControls]);

  const handleScroll = React.useCallback(
    (event: React.UIEvent<HTMLDivElement>) => {
      updateControls(event.currentTarget.scrollLeft);
    },
    [updateControls],
  );

  const scrollByPage = React.useCallback((direction: 1 | -1) => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }

    element.scrollBy({
      left: direction * Math.max(element.clientWidth * 0.8, 240),
      behavior: "smooth",
    });
  }, []);

  React.useEffect(() => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }

    measureOverflow();
    if (typeof ResizeObserver === "undefined") {
      return;
    }

    const resizeObserver = new ResizeObserver(measureOverflow);
    resizeObserver.observe(element);
    return () => resizeObserver.disconnect();
  }, [children, measureOverflow]);

  return (
    <div className={cn("relative min-w-0", containerClassName)}>
      <div ref={scrollRef} className={className} onScroll={handleScroll}>
        {children}
      </div>
      {overflow.hasMoreLeft ? (
        <>
          <div
            aria-hidden="true"
            className={cn(
              "pointer-events-none absolute inset-y-0 left-0 z-10 w-14 bg-gradient-to-l from-transparent to-[var(--scry-surf)]",
              fadeClassName,
            )}
          />
          <button
            type="button"
            aria-label={scrollBackControlLabel}
            onClick={() => scrollByPage(-1)}
            style={{ borderColor: "var(--scry-ink2)" }}
            className="absolute top-1/2 left-3 z-20 inline-flex size-14 -translate-y-1/2 items-center justify-center rounded-full border-[3px] border-[var(--scry-ink2)] bg-[var(--scry-bg)] text-[var(--scry-ink2)] shadow-[0_8px_28px_rgba(0,0,0,0.42),inset_0_1px_0_rgba(255,255,255,0.12)] transition-colors hover:bg-[var(--scry-card2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)]"
          >
            <ChevronLeft className="size-7 stroke-[3]" aria-hidden="true" />
          </button>
        </>
      ) : null}
      {overflow.hasMoreRight ? (
        <>
          <div
            aria-hidden="true"
            className={cn(
              "pointer-events-none absolute inset-y-0 right-0 z-10 w-14 bg-gradient-to-r from-transparent to-[var(--scry-surf)]",
              fadeClassName,
            )}
          />
          <button
            type="button"
            aria-label={scrollControlLabel}
            onClick={() => scrollByPage(1)}
            style={{ borderColor: "var(--scry-ink2)" }}
            className="absolute top-1/2 right-3 z-20 inline-flex size-14 -translate-y-1/2 items-center justify-center rounded-full border-[3px] border-[var(--scry-ink2)] bg-[var(--scry-bg)] text-[var(--scry-ink2)] shadow-[0_8px_28px_rgba(0,0,0,0.42),inset_0_1px_0_rgba(255,255,255,0.12)] transition-colors hover:bg-[var(--scry-card2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-accent-ring)]"
          >
            <ChevronRight className="size-7 stroke-[3]" aria-hidden="true" />
          </button>
        </>
      ) : null}
    </div>
  );
}

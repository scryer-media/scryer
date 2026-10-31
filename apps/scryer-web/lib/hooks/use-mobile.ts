import * as React from "react";

const MOBILE_BREAKPOINT = 768;
export const ROOT_SHELL_MOBILE_BREAKPOINT = 981;

export function useIsMobile(breakpoint = MOBILE_BREAKPOINT) {
  const [isMobile, setIsMobile] = React.useState<boolean | undefined>(() =>
    typeof window === "undefined" ? undefined : window.innerWidth < breakpoint,
  );

  React.useEffect(() => {
    const mql = window.matchMedia(`(max-width: ${breakpoint - 1}px)`);
    const onChange = () => {
      setIsMobile(window.innerWidth < breakpoint);
    };
    mql.addEventListener("change", onChange);
    setIsMobile(window.innerWidth < breakpoint);
    return () => mql.removeEventListener("change", onChange);
  }, [breakpoint]);

  return !!isMobile;
}

/**
 * Like useIsMobile, but measures an element instead of the viewport, so a
 * layout inside a narrow panel switches even when the window is wide. Falls
 * back to the viewport until the element has been measured.
 */
export function useIsNarrowElement(
  ref: React.RefObject<HTMLElement | null>,
  breakpoint = MOBILE_BREAKPOINT,
) {
  const viewportIsNarrow = useIsMobile(breakpoint);
  const [isNarrow, setIsNarrow] = React.useState<boolean | undefined>(
    undefined,
  );

  React.useLayoutEffect(() => {
    const element = ref.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const measure = () => {
      setIsNarrow(element.getBoundingClientRect().width < breakpoint);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref, breakpoint]);

  return isNarrow ?? viewportIsNarrow;
}

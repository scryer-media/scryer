export type FloatingPanelPlacement = {
  top: number;
  left: number;
  width: number;
  maxHeight: number;
  side: "top" | "bottom";
};

type ResolveFloatingPanelPlacementArgs = {
  anchorRect: DOMRect;
  desiredWidth: number;
  desiredMaxHeight: number;
  offset?: number;
  viewportPadding?: number;
};

export function resolveFloatingPanelPlacement({
  anchorRect,
  desiredWidth,
  desiredMaxHeight,
  offset = 4,
  viewportPadding = 8,
}: ResolveFloatingPanelPlacementArgs): FloatingPanelPlacement {
  const viewportWidth = typeof window === "undefined" ? desiredWidth : window.innerWidth;
  const viewportHeight = typeof window === "undefined" ? desiredMaxHeight : window.innerHeight;

  const availableBelow = Math.max(
    0,
    viewportHeight - anchorRect.bottom - offset - viewportPadding,
  );
  const availableAbove = Math.max(0, anchorRect.top - offset - viewportPadding);

  const side =
    availableBelow >= desiredMaxHeight || availableBelow >= availableAbove
      ? "bottom"
      : "top";
  const availableHeight = side === "bottom" ? availableBelow : availableAbove;
  const maxHeight = Math.min(desiredMaxHeight, Math.max(availableHeight, 0));

  const width = Math.min(
    desiredWidth,
    Math.max(240, viewportWidth - viewportPadding * 2),
  );
  const maxLeft = Math.max(viewportPadding, viewportWidth - viewportPadding - width);
  const left = Math.min(Math.max(anchorRect.left, viewportPadding), maxLeft);
  const top =
    side === "bottom"
      ? anchorRect.bottom + offset
      : Math.max(viewportPadding, anchorRect.top - offset - maxHeight);

  return {
    top,
    left,
    width,
    maxHeight,
    side,
  };
}

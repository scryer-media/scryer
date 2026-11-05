/** Use rendered CSS-pixel sizes: camera ratios vary with layout and viewport. */
export function showOverviewTitleLabels(
  visibleNodes: readonly { r: number }[],
  width: number,
  height: number,
): boolean {
  return (
    visibleNodes.length > 0 &&
    visibleNodes.length <= Math.min(80, (width * height) / 12_000) &&
    visibleNodes.filter(({ r }) => r >= 20).length >= visibleNodes.length / 2
  );
}

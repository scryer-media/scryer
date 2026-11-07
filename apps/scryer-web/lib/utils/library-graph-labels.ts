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

export type LabelRect = {
  left: number;
  top: number;
  right: number;
  bottom: number;
};

/**
 * The rectangles of one frame bucketed into a uniform screen grid, so asking
 * whether a spot overlaps any of them reads only the cells under the spot.
 * A neighbourhood name tries up to 161 spots and a zoomed-in view holds
 * thousands of titles; scanning every title for every spot does not fit in a
 * frame. Rectangles past the frame edge fall into the edge cells, so the
 * answer is exact for any box.
 */
export function createRectIndex(
  rects: readonly LabelRect[],
  width: number,
  height: number,
  cellSize = 64,
) {
  const columns = Math.max(1, Math.ceil(width / cellSize));
  const rows = Math.max(1, Math.ceil(height / cellSize));
  const cells: number[][] = Array.from({ length: columns * rows }, () => []);
  const column = (x: number) =>
    Math.min(columns - 1, Math.max(0, Math.floor(x / cellSize)));
  const row = (y: number) =>
    Math.min(rows - 1, Math.max(0, Math.floor(y / cellSize)));
  rects.forEach((rect, index) => {
    for (let r = row(rect.top); r <= row(rect.bottom); r += 1)
      for (let c = column(rect.left); c <= column(rect.right); c += 1)
        cells[r * columns + c].push(index);
  });
  return {
    /** Whether any rectangle overlaps the open box, edges not counting. */
    overlaps(left: number, top: number, right: number, bottom: number) {
      for (let r = row(top); r <= row(bottom); r += 1)
        for (let c = column(left); c <= column(right); c += 1)
          for (const index of cells[r * columns + c]) {
            const rect = rects[index];
            if (
              rect.left < right &&
              rect.right > left &&
              rect.top < bottom &&
              rect.bottom > top
            )
              return true;
          }
      return false;
    },
  };
}

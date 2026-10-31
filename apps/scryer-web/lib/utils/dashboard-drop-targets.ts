/** A rectangle in grid cells: columns across, rows down. */
export type GridRect = { x: number; y: number; w: number; h: number };

/**
 * Where a panel of `size` can land among `others` on a grid `columns` wide:
 * the cells covered by at least one position where the whole panel fits
 * without overlapping another panel. Panels never collide on the dashboard,
 * so this is exactly the area a drop can reach. The open rows below every
 * panel are included, one panel height deep, or down to `minRows` when that
 * is further.
 *
 * The area comes back as rectangles, each a run of free columns merged with
 * the identical runs in the rows below it.
 */
export function dashboardDropRegions(
  others: readonly GridRect[],
  size: { w: number; h: number },
  columns: number,
  minRows = 0,
): GridRect[] {
  const w = Math.min(Math.max(1, size.w), columns);
  const h = Math.max(1, size.h);
  const bottom = others.reduce(
    (max, rect) => Math.max(max, rect.y + rect.h),
    0,
  );
  const rows = Math.max(bottom + h, minRows);

  const occupied = Array.from({ length: rows }, () =>
    new Array<boolean>(columns).fill(false),
  );
  for (const rect of others) {
    for (
      let y = Math.max(0, rect.y);
      y < Math.min(rows, rect.y + rect.h);
      y += 1
    ) {
      for (
        let x = Math.max(0, rect.x);
        x < Math.min(columns, rect.x + rect.w);
        x += 1
      ) {
        occupied[y][x] = true;
      }
    }
  }

  const reachable = Array.from({ length: rows }, () =>
    new Array<boolean>(columns).fill(false),
  );
  for (let y = 0; y + h <= rows; y += 1) {
    for (let x = 0; x + w <= columns; x += 1) {
      if (!fits(occupied, x, y, w, h)) {
        continue;
      }
      for (let dy = 0; dy < h; dy += 1) {
        for (let dx = 0; dx < w; dx += 1) {
          reachable[y + dy][x + dx] = true;
        }
      }
    }
  }

  return mergeRuns(reachable, columns);
}

/**
 * The free position nearest to `rect` for a panel of its size among `others`
 * on a grid `columns` wide, or `rect`'s own position when it is already free.
 * Distance is measured on screen: `cellAspect` is a column step's width over a
 * row step's height, so a column away counts as far as it looks. The rows
 * below every panel are always free, so there is always an answer.
 */
export function nearestOpenSpot(
  others: readonly GridRect[],
  rect: GridRect,
  columns: number,
  cellAspect = 1,
): { x: number; y: number } {
  const w = Math.min(Math.max(1, rect.w), columns);
  const h = Math.max(1, rect.h);
  const bottom = others.reduce(
    (max, other) => Math.max(max, other.y + other.h),
    0,
  );
  const rows = Math.max(bottom, rect.y) + h;
  const occupied = Array.from({ length: rows }, () =>
    new Array<boolean>(columns).fill(false),
  );
  for (const other of others) {
    for (
      let y = Math.max(0, other.y);
      y < Math.min(rows, other.y + other.h);
      y += 1
    ) {
      for (
        let x = Math.max(0, other.x);
        x < Math.min(columns, other.x + other.w);
        x += 1
      ) {
        occupied[y][x] = true;
      }
    }
  }

  let best = {
    x: Math.min(Math.max(0, rect.x), columns - w),
    y: Math.max(0, bottom),
  };
  let bestDistance = Number.POSITIVE_INFINITY;
  for (let y = 0; y + h <= rows; y += 1) {
    for (let x = 0; x + w <= columns; x += 1) {
      const dx = (x - rect.x) * cellAspect;
      const dy = y - rect.y;
      const distance = dx * dx + dy * dy;
      if (distance < bestDistance && fits(occupied, x, y, w, h)) {
        best = { x, y };
        bestDistance = distance;
      }
    }
  }
  return best;
}

function fits(
  occupied: boolean[][],
  x: number,
  y: number,
  w: number,
  h: number,
): boolean {
  for (let dy = 0; dy < h; dy += 1) {
    for (let dx = 0; dx < w; dx += 1) {
      if (occupied[y + dy][x + dx]) {
        return false;
      }
    }
  }
  return true;
}

/** Row runs of reachable cells, each extended down while the next row repeats it. */
function mergeRuns(reachable: boolean[][], columns: number): GridRect[] {
  const done: GridRect[] = [];
  let open = new Map<string, GridRect>();
  for (let y = 0; y < reachable.length; y += 1) {
    const next = new Map<string, GridRect>();
    let x = 0;
    while (x < columns) {
      if (!reachable[y][x]) {
        x += 1;
        continue;
      }
      const start = x;
      while (x < columns && reachable[y][x]) {
        x += 1;
      }
      const key = `${start}:${x}`;
      const continued = open.get(key);
      if (continued) {
        continued.h += 1;
        next.set(key, continued);
        open.delete(key);
      } else {
        next.set(key, { x: start, y, w: x - start, h: 1 });
      }
    }
    done.push(...open.values());
    open = next;
  }
  done.push(...open.values());
  return done.sort((a, b) => a.y - b.y || a.x - b.x);
}

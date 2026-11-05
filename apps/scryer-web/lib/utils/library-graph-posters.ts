export const GRAPH_POSTER_ASPECT_RATIO = 2 / 3;
export const GRAPH_POSTER_CORNER_RADIUS_RATIO = 0.12;

type Point = { x: number; y: number };

/** Bounds for a poster whose size is its half-height in viewport pixels. */
export function graphPosterBounds(point: Point, size: number, padding = 0) {
  const halfWidth = size * GRAPH_POSTER_ASPECT_RATIO + padding;
  const halfHeight = size + padding;
  return {
    left: point.x - halfWidth,
    top: point.y - halfHeight,
    right: point.x + halfWidth,
    bottom: point.y + halfHeight,
    width: halfWidth * 2,
    height: halfHeight * 2,
    cornerRadius: size * GRAPH_POSTER_CORNER_RADIUS_RATIO + padding,
  };
}

export type GraphPosterBounds = ReturnType<typeof graphPosterBounds>;

export function graphPosterHoverable(
  size: number,
  opacity: number,
  loaded: boolean,
) {
  return loaded && size >= 9 && opacity >= 0.5;
}

/** Relations emerge after dots become posters, without exposing a mesh at once. */
export function graphRelationOpacity(size: number, detailOpacity = 1) {
  const progress = Math.max(0, Math.min(1, (size - 9) / 15));
  return (
    Math.round(progress * progress * (3 - 2 * progress) * detailOpacity * 64) /
    64
  );
}

/** Refresh only edges incident to viewport crossings or changed fade levels. */
export function changedGraphViewportEdges(
  previous: ReadonlySet<string>,
  next: ReadonlySet<string>,
  edgesForNode: (key: string) => readonly string[],
  changed: Iterable<string> = [],
) {
  const edges = new Set<string>();
  for (const key of previous) {
    if (!next.has(key)) for (const edge of edgesForNode(key)) edges.add(edge);
  }
  for (const key of next) {
    if (!previous.has(key))
      for (const edge of edgesForNode(key)) edges.add(edge);
  }
  for (const key of changed)
    for (const edge of edgesForNode(key)) edges.add(edge);
  return [...edges];
}

export function graphPosterInViewport(
  point: Point,
  size: number,
  width: number,
  height: number,
) {
  if (width <= 0 || height <= 0) return false;
  const box = graphPosterBounds(point, size);
  return (
    box.right >= 0 && box.left <= width && box.bottom >= 0 && box.top <= height
  );
}

/** Admission precedes image loading, so pending requests count toward the
 * viewport budget just like decoded images. Create once per frame. */
export function createGraphPosterAdmission(
  width: number,
  height: number,
  minSize: number,
  limit: number,
) {
  let admitted = 0;
  return (point: Point, size: number): boolean => {
    if (
      admitted >= limit ||
      size < minSize ||
      !graphPosterInViewport(point, size, width, height)
    )
      return false;
    admitted += 1;
    return true;
  };
}

/** Keep the visible working set, plus a bounded offscreen cache. Requests are
 * admitted separately from drawing so a busy viewport can finish every poster. */
export function prepareGraphPosterImages(
  cache: Map<string, HTMLImageElement>,
  visible: ReadonlySet<string>,
  onChange: () => void,
  createImage: () => HTMLImageElement = () => new Image(),
  maxPending = 12,
  maxOffscreen = 240,
) {
  let offscreen = [...cache.keys()].filter((url) => !visible.has(url)).length;
  let pending = 0;
  for (const [url, image] of cache) {
    if (!visible.has(url) && (!image.complete || offscreen > maxOffscreen)) {
      image.onload = null;
      image.onerror = null;
      if (!image.complete) image.src = "";
      cache.delete(url);
      offscreen--;
    } else if (!image.complete) {
      pending++;
    }
  }
  for (const url of visible) {
    const cached = cache.get(url);
    if (cached) {
      cache.delete(url);
      cache.set(url, cached);
      continue;
    }
    if (pending >= maxPending) continue;
    const image = createImage();
    image.decoding = "async";
    image.onload = onChange;
    image.onerror = onChange;
    cache.set(url, image);
    image.src = url;
    pending++;
  }
}

export function traceGraphPoster(
  context: CanvasRenderingContext2D,
  point: Point,
  size: number,
  padding = 0,
) {
  const box = graphPosterBounds(point, size, padding);
  context.beginPath();
  context.roundRect(box.left, box.top, box.width, box.height, box.cornerRadius);
}

export function drawGraphPoster(
  context: CanvasRenderingContext2D,
  image: HTMLImageElement,
  point: Point,
  size: number,
) {
  const box = graphPosterBounds(point, size);
  const scale = Math.max(
    box.width / image.naturalWidth,
    box.height / image.naturalHeight,
  );
  const width = image.naturalWidth * scale;
  const height = image.naturalHeight * scale;
  context.save();
  traceGraphPoster(context, point, size);
  context.clip();
  context.drawImage(
    image,
    point.x - width / 2,
    point.y - height / 2,
    width,
    height,
  );
  context.restore();
}

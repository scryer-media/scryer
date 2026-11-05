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
    if (admitted >= limit || size < minSize || width <= 0 || height <= 0)
      return false;
    const box = graphPosterBounds(point, size);
    if (box.right < 0 || box.left > width || box.bottom < 0 || box.top > height)
      return false;
    admitted += 1;
    return true;
  };
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

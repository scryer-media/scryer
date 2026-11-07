import {
  selectPosterVariantUrl,
  type LocalPosterVariant,
} from "./poster-images.ts";

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

/** Artwork follows visible poster size, independently of the zone crossfade. */
export function graphPosterArtworkOpacity(size: number, minSize = 9) {
  if (minSize === 0) return 1;
  const t = Math.max(0, Math.min(1, (size - minSize * 0.7) / (minSize * 0.6)));
  return t * t * (3 - 2 * t);
}

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

// Stored poster heights, in pixels; `original` is whatever the source was.
const GRAPH_POSTER_VARIANT_HEIGHTS: readonly [LocalPosterVariant, number][] = [
  ["w70", 102],
  ["w250", 367],
];

/** The smallest stored variant that covers the poster's drawn height on the
 * device, allowing a slight upscale before stepping to the next one. */
export function graphPosterVariant(
  size: number,
  pixelRatio: number,
): LocalPosterVariant {
  const height = size * 2 * (pixelRatio > 0 ? pixelRatio : 1);
  for (const [variant, stored] of GRAPH_POSTER_VARIANT_HEIGHTS)
    if (height <= stored * 1.25) return variant;
  return "original";
}

// Variant addresses are resolved once per poster; parsing them on every frame
// costs milliseconds per frame at a few hundred posters.
const variantUrls = new Map<string, Record<LocalPosterVariant, string>>();

/** A poster's address at each stored variant. */
export function graphPosterVariantUrls(
  url: string,
): Readonly<Record<LocalPosterVariant, string>> {
  let urls = variantUrls.get(url);
  if (!urls) {
    urls = {
      w70: selectPosterVariantUrl(url, "w70") ?? url,
      w250: selectPosterVariantUrl(url, "w250") ?? url,
      original: selectPosterVariantUrl(url, "original") ?? url,
    };
    variantUrls.set(url, urls);
  }
  return urls;
}

/** The poster URL to load at this size, then the other variants, largest
 * first, any one of which can stand in while the wanted one loads. */
export function graphPosterUrls(
  url: string,
  size: number,
  pixelRatio: number,
): string[] {
  const variants = graphPosterVariantUrls(url);
  const urls = [variants[graphPosterVariant(size, pixelRatio)]];
  for (const other of [variants.original, variants.w250, variants.w70])
    if (!urls.includes(other)) urls.push(other);
  return urls;
}

/** Whether a poster URL names the full-size source image. */
export function isOriginalGraphPosterUrl(url: string): boolean {
  return /\/original(?:[?#]|$)/.test(url);
}

export function isThumbnailGraphPosterUrl(url: string): boolean {
  return /\/w70(?:[?#]|$)/.test(url);
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

export type GraphPosterCacheLimits = {
  /** Full-size images, several times the pixels of the others. */
  isLarge?: (url: string) => boolean;
  maxOffscreenLarge?: number;
  /** The thumbnails every poster falls back to while a zoom moves. */
  isThumbnail?: (url: string) => boolean;
  maxOffscreenThumbnails?: number;
};

/** Keep the visible working set, plus a bounded offscreen cache. Requests are
 * admitted separately from drawing so a busy viewport can finish every poster.
 * Full-size images keep a smaller offscreen allowance of their own, and
 * thumbnails a much larger one: they are a small fraction of the pixels, and
 * a poster drawn from a sharper variant leaves its thumbnail offscreen, so
 * sharing one allowance would drop every thumbnail in a zoomed-in view and
 * load them all again on the way back out. */
export function prepareGraphPosterImages(
  cache: Map<string, HTMLImageElement>,
  visible: ReadonlySet<string>,
  onChange: () => void,
  createImage: () => HTMLImageElement = () => new Image(),
  maxPending = 12,
  maxOffscreen = 240,
  {
    isLarge = () => false,
    maxOffscreenLarge = 16,
    isThumbnail = () => false,
    maxOffscreenThumbnails = 2000,
  }: GraphPosterCacheLimits = {},
) {
  const pool = (url: string) =>
    isThumbnail(url) ? "thumbnail" : isLarge(url) ? "large" : "standard";
  const limits = {
    thumbnail: maxOffscreenThumbnails,
    large: maxOffscreenLarge,
    standard: maxOffscreen,
  };
  const offscreen = { thumbnail: 0, large: 0, standard: 0 };
  for (const url of cache.keys()) {
    if (!visible.has(url)) offscreen[pool(url)]++;
  }
  let pending = 0;
  for (const [url, image] of cache) {
    if (visible.has(url)) {
      if (!image.complete) pending++;
      continue;
    }
    const kind = pool(url);
    if (!image.complete || offscreen[kind] > limits[kind]) {
      image.onload = null;
      image.onerror = null;
      if (!image.complete) image.src = "";
      cache.delete(url);
      offscreen[kind]--;
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

type SpriteCanvas = {
  width: number;
  height: number;
  getContext(kind: "2d"): CanvasRenderingContext2D | null;
};

// The shared rounded shapes are drawn once at this size and scaled per poster.
const SHAPE_SPRITE_HEIGHT = 384;

function roundedSprite(
  createCanvas: () => SpriteCanvas,
  width: number,
  height: number,
  paint: (context: CanvasRenderingContext2D) => void,
) {
  const canvas = createCanvas();
  canvas.width = Math.max(1, Math.round(width));
  canvas.height = Math.max(1, Math.round(height));
  const context = canvas.getContext("2d");
  if (context) {
    context.beginPath();
    context.roundRect(
      0,
      0,
      canvas.width,
      canvas.height,
      (canvas.height / 2) * GRAPH_POSTER_CORNER_RADIUS_RATIO,
    );
    context.clip();
    paint(context);
  }
  return canvas;
}

type DecodedImage = {
  source: CanvasImageSource;
  width: number;
  height: number;
};

// `decode` works off the main thread; `createImageBitmap` of an image
// element decodes on it, about a millisecond a poster.
function decodeOffThread(image: HTMLImageElement): Promise<DecodedImage> {
  return image.decode().then(() => ({
    source: image,
    width: image.naturalWidth,
    height: image.naturalHeight,
  }));
}

/**
 * Posters pre-cut to their rounded shape. Clipping each poster as it is drawn
 * costs a clip and a restore per poster per frame, which in a zoomed-in view
 * of a few hundred posters takes most of the frame. Cut once, a poster is
 * three plain image draws. Artwork is decoded off the main thread before it
 * is cut, so a poster arriving mid-zoom never stalls a frame; `onReady` asks
 * for the frame that shows it.
 */
export function createGraphPosterSprites(
  onReady: () => void,
  createCanvas: () => SpriteCanvas = () => document.createElement("canvas"),
  decode: (image: HTMLImageElement) => Promise<DecodedImage> = decodeOffThread,
) {
  const shapeWidth = SHAPE_SPRITE_HEIGHT * GRAPH_POSTER_ASPECT_RATIO;
  let mask: SpriteCanvas | null = null;
  let backing: { color: string; canvas: SpriteCanvas } | null = null;
  const artwork = new WeakMap<HTMLImageElement, SpriteCanvas | "decoding">();
  const cut = (image: DecodedImage) => {
    // The image's own resolution, cropped to fill the poster's shape.
    const height = image.height;
    const width = height * GRAPH_POSTER_ASPECT_RATIO;
    return roundedSprite(createCanvas, width, height, (context) => {
      const scale = Math.max(width / image.width, height / image.height);
      const drawnWidth = image.width * scale;
      const drawnHeight = image.height * scale;
      context.drawImage(
        image.source,
        (width - drawnWidth) / 2,
        (height - drawnHeight) / 2,
        drawnWidth,
        drawnHeight,
      );
    });
  };
  return {
    mask() {
      mask ??= roundedSprite(
        createCanvas,
        shapeWidth,
        SHAPE_SPRITE_HEIGHT,
        (context) => {
          context.fillStyle = "#000";
          context.fillRect(0, 0, shapeWidth, SHAPE_SPRITE_HEIGHT);
        },
      );
      return mask;
    },
    backing(color: string) {
      if (backing?.color !== color) {
        backing = {
          color,
          canvas: roundedSprite(
            createCanvas,
            shapeWidth,
            SHAPE_SPRITE_HEIGHT,
            (context) => {
              context.fillStyle = color;
              context.fillRect(0, 0, shapeWidth, SHAPE_SPRITE_HEIGHT);
            },
          ),
        };
      }
      return backing.canvas;
    },
    /** The cut artwork, or null while the image is still being decoded. */
    artwork(image: HTMLImageElement): SpriteCanvas | null {
      const sprite = artwork.get(image);
      if (sprite === "decoding") return null;
      if (sprite) return sprite;
      artwork.set(image, "decoding");
      decode(image).then(
        (decoded) => {
          artwork.set(image, cut(decoded));
          onReady();
        },
        // An image that cannot be decoded is drawn without a poster.
        () => undefined,
      );
      return null;
    },
  };
}

export type GraphPosterSprites = ReturnType<typeof createGraphPosterSprites>;

/**
 * Draws a poster at the context's opacity, once its artwork is ready, and
 * says whether it did. The context's opacity and compositing are left as
 * they were.
 */
export function drawGraphPoster(
  context: CanvasRenderingContext2D,
  sprites: GraphPosterSprites,
  image: HTMLImageElement,
  point: Point,
  size: number,
  background: string,
) {
  const opacity = context.globalAlpha;
  if (opacity <= 0) return false;
  const artwork = sprites.artwork(image);
  if (!artwork) return false;
  const box = graphPosterBounds(point, size);
  const composite = context.globalCompositeOperation;
  // Theme surfaces can be translucent. Remove zones and relation lines
  // under the poster's shape so they cannot bleed through fading artwork.
  context.globalAlpha = 1;
  context.globalCompositeOperation = "destination-out";
  context.drawImage(
    sprites.mask() as CanvasImageSource,
    box.left,
    box.top,
    box.width,
    box.height,
  );
  context.globalCompositeOperation = composite;
  context.drawImage(
    sprites.backing(background) as CanvasImageSource,
    box.left,
    box.top,
    box.width,
    box.height,
  );
  context.globalAlpha = opacity;
  context.drawImage(
    artwork as CanvasImageSource,
    box.left,
    box.top,
    box.width,
    box.height,
  );
  return true;
}

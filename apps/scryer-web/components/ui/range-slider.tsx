import { cn } from "@/lib/utils";

/**
 * The filter-rail slider look shared with the catalog and discovery filter
 * panels: a muted track, an accent fill and white thumbs.
 */
const RANGE_SLIDER_INPUT_CLASS_NAME =
  "h-1.5 w-full appearance-none rounded-full bg-transparent accent-[var(--scry-accent)] outline-none [&::-moz-range-progress]:h-1.5 [&::-moz-range-progress]:rounded-full [&::-moz-range-progress]:bg-transparent [&::-moz-range-thumb]:h-[15px] [&::-moz-range-thumb]:w-[15px] [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-0 [&::-moz-range-thumb]:bg-white [&::-moz-range-thumb]:shadow-[0_1px_5px_rgba(0,0,0,0.5)] [&::-moz-range-track]:h-1.5 [&::-moz-range-track]:rounded-full [&::-moz-range-track]:bg-transparent [&::-webkit-slider-runnable-track]:h-1.5 [&::-webkit-slider-runnable-track]:rounded-full [&::-webkit-slider-runnable-track]:bg-transparent [&::-webkit-slider-thumb]:mt-[-4.5px] [&::-webkit-slider-thumb]:h-[15px] [&::-webkit-slider-thumb]:w-[15px] [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-white [&::-webkit-slider-thumb]:shadow-[0_1px_5px_rgba(0,0,0,0.5)] focus-visible:[&::-webkit-slider-thumb]:ring-2 focus-visible:[&::-webkit-slider-thumb]:ring-[var(--scry-accent-ring)] focus-visible:[&::-moz-range-thumb]:ring-2 focus-visible:[&::-moz-range-thumb]:ring-[var(--scry-accent-ring)]";

/** Lets two stacked inputs share one track: only the thumbs take the pointer. */
const THUMB_ONLY_POINTER_CLASS_NAME =
  "pointer-events-none [&::-moz-range-thumb]:pointer-events-auto [&::-webkit-slider-thumb]:pointer-events-auto";

function percentOf(value: number, min: number, max: number): number {
  if (max <= min) {
    return 0;
  }
  return Math.min(100, Math.max(0, ((value - min) / (max - min)) * 100));
}

type RangeSliderThumb = {
  id: string;
  ariaLabel: string;
};

type RangeSliderProps = {
  min: number;
  max: number;
  step?: number;
  value: [number, number];
  onValueChange: (value: [number, number]) => void;
  lowThumb: RangeSliderThumb;
  highThumb: RangeSliderThumb;
  className?: string;
};

/**
 * A two-thumb range on one track. Each thumb is a native range input, so
 * keyboard, form and assistive behaviour are the browser's; the thumbs cannot
 * cross.
 */
function RangeSlider({
  min,
  max,
  step = 1,
  value,
  onValueChange,
  lowThumb,
  highThumb,
  className,
}: RangeSliderProps) {
  const [low, high] = value;
  const inputClassName = cn(
    "absolute inset-x-0 top-1/2 -translate-y-1/2",
    RANGE_SLIDER_INPUT_CLASS_NAME,
    THUMB_ONLY_POINTER_CLASS_NAME,
  );
  return (
    <div data-slot="range-slider" className={cn("relative h-5", className)}>
      <div className="absolute inset-x-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-[var(--scry-border2)]" />
      <div
        className="absolute top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-gradient-to-r from-[var(--scry-accent)] to-[var(--scry-accent-ring)]"
        style={{
          left: `${percentOf(low, min, max)}%`,
          right: `${100 - percentOf(high, min, max)}%`,
        }}
      />
      <input
        id={lowThumb.id}
        type="range"
        aria-label={lowThumb.ariaLabel}
        min={min}
        max={max}
        step={step}
        value={low}
        onChange={(event) =>
          onValueChange([Math.min(Number(event.target.value), high), high])
        }
        // Once the low thumb is past the middle it sits on top, so two thumbs
        // parked together at the top end can still be pulled apart.
        style={{ zIndex: low > (min + max) / 2 ? 2 : 1 }}
        className={inputClassName}
      />
      <input
        id={highThumb.id}
        type="range"
        aria-label={highThumb.ariaLabel}
        min={min}
        max={max}
        step={step}
        value={high}
        onChange={(event) =>
          onValueChange([low, Math.max(Number(event.target.value), low)])
        }
        style={{ zIndex: low > (min + max) / 2 ? 1 : 2 }}
        className={inputClassName}
      />
    </div>
  );
}

export { RangeSlider };
export type { RangeSliderProps, RangeSliderThumb };

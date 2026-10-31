import { cn } from "@/lib/utils";
import { usageTone } from "@/lib/utils/dashboard";
import { usageToneStyle } from "./usage-tone-style";

/**
 * A small ring showing how full one storage root is.
 *
 * This is the dashboard's only genuinely new visual primitive. The app renders
 * proportions as linear bars (`Progress`, `ActivityProgressBar`), and a row of
 * bars cannot carry a dozen roots in the space this panel has: bars need a full
 * row each, whereas a 32px ring reads at a glance next to two lines of text and
 * lets the roots wrap as tiles. It is deliberately a leaf with no logic of its
 * own: the threshold comes from the shared `usageTone` ramp and every colour is
 * an existing `--scry-*` token, so it re-tints with the theme like everything
 * else.
 *
 * Rendered as a conic gradient with the centre punched out by a `background`
 * disc, so it costs one element rather than an SVG.
 */
export function StorageUsageRing({
  percent,
  className,
}: {
  /** Fill percentage, or null when the filesystem could not be inspected. */
  percent: number | null;
  className?: string;
}) {
  const known = percent !== null && Number.isFinite(percent);
  const clamped = known ? Math.max(0, Math.min(100, percent)) : 0;
  const tone = usageToneStyle(usageTone(clamped).tone);
  // An unknown root shows an empty track: an unreadable mount must never look
  // like a healthy, near-empty one.
  const fill = known ? tone.solid : "var(--scry-border2)";
  const track = known ? `rgba(${tone.rgb}, 0.16)` : "var(--scry-line)";

  return (
    <span
      aria-hidden="true"
      className={cn(
        "relative inline-block h-8 w-8 shrink-0 rounded-full",
        className,
      )}
      style={{
        background: `conic-gradient(${fill} 0 ${clamped}%, ${track} ${clamped}% 100%)`,
      }}
    >
      <span className="absolute inset-[5px] rounded-full bg-[var(--scry-surf)]" />
    </span>
  );
}

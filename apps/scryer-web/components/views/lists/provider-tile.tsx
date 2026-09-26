import type { ListProviderManifest } from "@/lib/types/lists";
import { providerTileAbbreviation, providerTileStyle } from "@/lib/utils/lists";
import { cn } from "@/lib/utils";

type ProviderTileProps = {
  provider: Pick<ListProviderManifest, "name" | "tile"> | null;
  size?: "sm" | "md";
  className?: string;
};

/**
 * A provider's brand square. Provider colours are the one hard-coded palette
 * this page uses; they arrive with the provider catalog.
 */
export function ProviderTile({ provider, size = "md", className }: ProviderTileProps) {
  const style = providerTileStyle(provider?.tile ?? null);
  return (
    <span
      aria-hidden="true"
      style={style ?? undefined}
      className={cn(
        "inline-flex flex-none select-none items-center justify-center rounded-[9px] font-display font-bold tracking-tight",
        size === "sm" ? "h-7 w-7 text-[11px]" : "h-10 w-10 text-[13px]",
        !style && "border border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[var(--scry-ink2)]",
        className,
      )}
    >
      {provider ? providerTileAbbreviation(provider) : "?"}
    </span>
  );
}

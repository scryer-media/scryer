import { useState } from "react";

import type { ListProviderManifest } from "@/lib/types/lists";
import { providerLogoSrc, providerTileAbbreviation, providerTileStyle } from "@/lib/utils/lists";
import { cn } from "@/lib/utils";

type ProviderTileProps = {
  provider: Pick<ListProviderManifest, "providerType" | "name" | "tile"> | null;
  size?: "sm" | "md";
  className?: string;
};

/**
 * A provider's brand square: the service's shipped logo on a neutral frame, or
 * the catalog's coloured abbreviation when no logo exists (or it fails to load).
 */
export function ProviderTile({ provider, size = "md", className }: ProviderTileProps) {
  const logoSrc = provider ? providerLogoSrc(provider.providerType) : null;
  const [failedLogoSrc, setFailedLogoSrc] = useState<string | null>(null);
  const showLogo = logoSrc !== null && logoSrc !== failedLogoSrc;
  const style = showLogo ? null : providerTileStyle(provider?.tile ?? null);
  return (
    <span
      aria-hidden="true"
      style={style ?? undefined}
      className={cn(
        "inline-flex flex-none select-none items-center justify-center overflow-hidden rounded-[9px] font-display font-bold tracking-tight",
        size === "sm" ? "h-7 w-7 text-[11px]" : "h-10 w-10 text-[13px]",
        showLogo && (size === "sm" ? "p-1" : "p-1.5"),
        showLogo
          ? "border border-[var(--scry-border2)] bg-[var(--scry-chip)]"
          : !style && "border border-[var(--scry-border2)] bg-[var(--scry-inset)] text-[var(--scry-ink2)]",
        className,
      )}
    >
      {showLogo ? (
        <img
          src={logoSrc}
          alt=""
          className="h-full w-full object-contain"
          loading="lazy"
          onError={() => setFailedLogoSrc(logoSrc)}
        />
      ) : provider ? (
        providerTileAbbreviation(provider)
      ) : (
        "?"
      )}
    </span>
  );
}

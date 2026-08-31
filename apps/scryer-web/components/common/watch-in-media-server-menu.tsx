import type { SyntheticEvent } from "react";

import { IconButton } from "@/components/ui/icon-button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";

export type MediaServerPlaybackLink = {
  connectionId: string;
  displayName: string;
  provider: "JELLYFIN" | "PLEX" | "EMBY";
  href: string;
};

const providerIconSrc: Record<MediaServerPlaybackLink["provider"], string> = {
  JELLYFIN: "/auth-providers/jellyfin.svg",
  PLEX: "/auth-providers/plex.svg",
  EMBY: "/auth-providers/emby.svg",
};

const providerLabel: Record<MediaServerPlaybackLink["provider"], string> = {
  JELLYFIN: "Jellyfin",
  PLEX: "Plex",
  EMBY: "Emby",
};

/** Direct provider links for a title or episode that passed playback authorization. */
export function WatchInMediaServerMenu({
  links,
  className,
  compact = false,
  showLabel = false,
}: {
  links?: MediaServerPlaybackLink[] | null;
  className?: string;
  compact?: boolean;
  showLabel?: boolean;
}) {
  const t = useTranslate();
  if (!links || links.length === 0) return null;

  const stopParentNavigation = (event: SyntheticEvent) => {
    event.stopPropagation();
  };
  const openPlaybackLink = (connectionId: string) => {
    const link = links.find((candidate) => candidate.connectionId === connectionId);
    if (!link) return;
    window.open(link.href, "_blank", "noopener,noreferrer");
  };

  if (links.length > 1) {
    return (
      <div
        role="group"
        aria-label={t("label.watchIn")}
        className={cn("flex items-center", className)}
      >
        <Select value="" onValueChange={openPlaybackLink}>
          <SelectTrigger
            size="sm"
            chrome="toolbar"
            aria-label={t("label.watchIn")}
            className="h-8 shrink-0 gap-1.5 border-[var(--scry-border2)] bg-[var(--scry-inset)] px-2.5 text-[11px] font-semibold text-[#dbe4fb]"
            onClick={stopParentNavigation}
            onPointerDown={stopParentNavigation}
          >
            <SelectValue placeholder={`${t("label.watchIn")}…`} />
          </SelectTrigger>
          <SelectContent position="popper" align="end" className="min-w-[12rem]">
            {links.map((link) => {
              const provider = providerLabel[link.provider];
              return (
                <SelectItem key={link.connectionId} value={link.connectionId}>
                  <span className="flex min-w-0 items-center gap-2">
                    <img
                      src={providerIconSrc[link.provider]}
                      alt=""
                      aria-hidden="true"
                      className="h-4 w-4 shrink-0 object-contain"
                    />
                    <span className="truncate">
                      {provider} — {link.displayName}
                    </span>
                  </span>
                </SelectItem>
              );
            })}
          </SelectContent>
        </Select>
      </div>
    );
  }

  return (
    <div
      role="group"
      aria-label={t("label.watchIn")}
      className={cn("flex flex-wrap items-center gap-1.5", className)}
    >
      {links.map((link) => {
        const provider = providerLabel[link.provider];
        const label = `${t("label.watchIn")} ${provider} — ${link.displayName}`;

        return (
          <IconButton
            key={link.connectionId}
            asChild
            label={label}
            tooltipSide="top"
            appearance={compact ? "ghost" : "boxed"}
            className={cn(
              "shrink-0 rounded-[9px] [&_img]:transition-transform [&:hover_img]:scale-105",
              showLabel
                ? "h-8 w-auto gap-2 px-2.5 text-[11px] font-semibold text-[#dbe4fb]"
                : compact
                  ? "h-7 w-7"
                  : "h-9 w-9",
            )}
          >
            <a
              href={link.href}
              target="_blank"
              rel="noopener noreferrer"
              onClick={stopParentNavigation}
              onPointerDown={stopParentNavigation}
            >
              <img
                src={providerIconSrc[link.provider]}
                alt=""
                aria-hidden="true"
                className={cn(
                  "object-contain",
                  compact ? "h-4 w-4" : "h-[19px] w-[19px]",
                )}
              />
              {showLabel ? (
                <span className="max-w-[9rem] truncate">
                  {t("label.watchIn")} {link.displayName}
                </span>
              ) : null}
            </a>
          </IconButton>
        );
      })}
    </div>
  );
}

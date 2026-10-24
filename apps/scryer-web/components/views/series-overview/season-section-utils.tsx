import * as React from "react";
import { IconButton } from "@/components/ui/icon-button";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import type { Release } from "@/lib/types";
import type { BoxedActionButtonTone } from "@/lib/utils/action-button-styles";
import type {
  EpisodeMediaFile,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";

export type TranslateFn = ReturnType<typeof useTranslate>;

export const EMPTY_EPISODE_FILES: EpisodeMediaFile[] = [];
export const EMPTY_RELEASES: Release[] = [];
export const EMPTY_SUBTITLE_DOWNLOADS: ExternalSubtitleRecord[] = [];
export const EMPTY_BLOCKLIST_ENTRIES: TitleReleaseBlocklistEntry[] = [];

export function EpisodeTableActionButton({
  label,
  tone,
  showTitleAttribute = true,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<typeof IconButton>, "tone"> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "auto" | "search">;
  showTitleAttribute?: boolean;
}) {
  return (
    <IconButton
      label={label}
      tone={tone}
      showTitleAttribute={showTitleAttribute}
      className={cn(
        "relative z-[1] bg-background hover:bg-background",
        tone === "auto"
          ? "bg-[linear-gradient(var(--scry-success-bg),var(--scry-success-bg))] hover:bg-[linear-gradient(var(--scry-success-bg-strong),var(--scry-success-bg-strong))]"
          : "bg-[linear-gradient(var(--scry-info-bg),var(--scry-info-bg))] hover:bg-[linear-gradient(var(--scry-info-bg-strong),var(--scry-info-bg-strong))]",
        className,
      )}
      {...props}
    >
      {children}
    </IconButton>
  );
}

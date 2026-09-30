import * as React from "react";
import {
  Archive,
  Bell,
  Download,
  Plug,
  Search,
  Server,
  Subtitles,
  type LucideIcon,
} from "lucide-react";

import { cn } from "@/lib/utils";
import {
  getPluginLogoSources,
  type PluginVisualIdentity,
} from "@/lib/utils/plugin-logos";

export function getPluginFallbackIcon(
  pluginType?: string | null,
): LucideIcon {
  const normalizedType = pluginType?.trim().toLowerCase() ?? "";
  if (normalizedType === "download_client") {
    return Download;
  }
  if (normalizedType === "archive_extractor") {
    return Archive;
  }
  if (normalizedType === "indexer" || normalizedType.endsWith("_indexer")) {
    return Search;
  }
  if (normalizedType === "notification") {
    return Bell;
  }
  if (normalizedType === "subtitle_provider") {
    return Subtitles;
  }
  if (normalizedType === "media_server") {
    return Server;
  }
  return Plug;
}

export function PluginLogo({
  id,
  name,
  providerType,
  pluginType,
  appearance = "framed",
  className,
  imageClassName,
  iconClassName,
}: PluginVisualIdentity & {
  appearance?: "framed" | "bare";
  className?: string;
  imageClassName?: string;
  iconClassName?: string;
}) {
  const sources = getPluginLogoSources({ id, name, providerType, pluginType });
  const [failedToLoadImage, setFailedToLoadImage] = React.useState(false);
  const FallbackIcon = getPluginFallbackIcon(pluginType);

  React.useEffect(() => {
    setFailedToLoadImage(false);
  }, [sources?.src]);

  return (
    <span
      className={cn(
        "inline-flex h-8 w-8 shrink-0 items-center justify-center self-center overflow-hidden text-[var(--scry-muted)]",
        appearance === "framed" &&
          "rounded-md border border-[var(--scry-border2)] bg-[var(--scry-chip)]",
        className,
      )}
      aria-hidden="true"
    >
      {sources && !failedToLoadImage ? (
        <picture className="flex h-full w-full items-center justify-center">
          {sources.svg ? (
            <source srcSet={sources.svg} type="image/svg+xml" />
          ) : null}
          {sources.avif ? (
            <source srcSet={sources.avif} type="image/avif" />
          ) : null}
          <img
            src={sources.src}
            alt=""
            className={cn("h-full w-full object-contain", imageClassName)}
            onError={() => setFailedToLoadImage(true)}
          />
        </picture>
      ) : (
        <FallbackIcon className={cn("m-auto h-4 w-4", iconClassName)} />
      )}
    </span>
  );
}

export function PluginVisualLabel({
  id,
  name,
  providerType,
  pluginType,
  label,
  className,
  logoClassName = "h-5 w-5 rounded-[6px]",
}: PluginVisualIdentity & {
  label: React.ReactNode;
  className?: string;
  logoClassName?: string;
}) {
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-2", className)}>
      <PluginLogo
        id={id}
        name={name}
        providerType={providerType}
        pluginType={pluginType}
        className={logoClassName}
        iconClassName="h-3.5 w-3.5"
      />
      <span className="truncate">{label}</span>
    </span>
  );
}

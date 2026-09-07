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

const PLUGIN_LOGO_BASE_PATH = "/plugin-logos";

const PLUGIN_LOGO_SVG_SLUGS = [
  "apprise",
  "aria2",
  "deluge",
  "discord",
  "flood",
  "gotify",
  "iptorrents",
  "mailgun",
  "emby",
  "notifiarr",
  "ntfy",
  "pushbullet",
  "pushover",
  "qbittorrent",
  "rqbit",
  "sendgrid",
  "signal",
  "slack",
  "synology",
  "telegram",
  "trakt",
  "transmission",
  "twitter",
  "utorrent",
  "whisper",
  "xbmc",
] as const;

const PLUGIN_LOGO_AVIF_SLUGS = [
  "broadcasthe-net",
  "downloadstation",
  "fanzub",
  "filelist",
  "flood",
  "hadouken",
  "jimaku",
  "join",
  "nzbgeek",
  "nzbvortex",
  "opensubtitles",
  "pneumatic",
  "prowl",
  "pushcut",
  "simplepush",
  "subdl",
  "torrentleech",
  "tribler",
] as const;

const LEGACY_PLUGIN_LOGO_SRC_BY_SLUG: Record<string, string> = {
  jellyfin: "/auth-providers/jellyfin.svg",
  nzbget: "/download-clients/nzbget.svg",
  plex: "/auth-providers/plex.svg",
  prowlarr: "/media-sites/prowlarr.svg",
  sabnzbd: "/download-clients/sabnzbd.svg",
  weaver: "/download-clients/weaver.webp",
};

const PLUGIN_LOGO_ALIASES: Record<string, string> = {
  "broadcasthe-net": "broadcasthe-net",
  broadcasthenet: "broadcasthe-net",
  "broadcasthenet-tv": "broadcasthe-net",
  "broadcasthenet-tv-tracker": "broadcasthe-net",
  btntv: "broadcasthe-net",
  "download-station": "downloadstation",
  "synology-download-station": "downloadstation",
  downloadstation: "downloadstation",
  "nzb-vortex": "nzbvortex",
  nzbvortex: "nzbvortex",
  "open-subtitles": "opensubtitles",
  "opensubtitles-com": "opensubtitles",
  "open-subtitles-com": "opensubtitles",
  opensubtitlescom: "opensubtitles",
  "qbit-torrent": "qbittorrent",
  "q-bit-torrent": "qbittorrent",
  qbit: "qbittorrent",
  "torrent-leech": "torrentleech",
  "u-torrent": "utorrent",
};

export type PluginVisualIdentity = {
  id?: string | null;
  name?: string | null;
  providerType?: string | null;
  pluginType?: string | null;
};

export type PluginLogoSources = {
  slug: string;
  svg?: string;
  avif?: string;
  src: string;
};

const svgLogoSlugs = new Set<string>(PLUGIN_LOGO_SVG_SLUGS);
const avifLogoSlugs = new Set<string>(PLUGIN_LOGO_AVIF_SLUGS);
const imageSlugByCompactSlug = new Map<string, string>();

for (const slug of [
  ...PLUGIN_LOGO_SVG_SLUGS,
  ...PLUGIN_LOGO_AVIF_SLUGS,
  ...Object.keys(LEGACY_PLUGIN_LOGO_SRC_BY_SLUG),
]) {
  imageSlugByCompactSlug.set(compactPluginSlug(slug), slug);
}

function slugifyPluginValue(value: string): string {
  return value
    .normalize("NFKD")
    .toLowerCase()
    .replace(/['’]/g, "")
    .replace(/&/g, " and ")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function compactPluginSlug(value: string): string {
  return slugifyPluginValue(value).replace(/-/g, "");
}

function candidateVariants(value: string): string[] {
  const slug = slugifyPluginValue(value);
  if (!slug) {
    return [];
  }

  return [
    slug,
    slug.replace(/^scryer-plugin-/, ""),
    slug.replace(/^plugin-/, ""),
    slug.replace(/-plugin$/, ""),
    compactPluginSlug(slug),
  ];
}

function hasPluginLogoSlug(slug: string): boolean {
  return (
    svgLogoSlugs.has(slug) ||
    avifLogoSlugs.has(slug) ||
    LEGACY_PLUGIN_LOGO_SRC_BY_SLUG[slug] !== undefined
  );
}

export function resolvePluginLogoSlug(
  identity: PluginVisualIdentity,
): string | null {
  const candidates = [
    identity.providerType,
    identity.id,
    identity.name,
  ].filter((value): value is string => Boolean(value?.trim()));

  for (const candidate of candidates) {
    for (const variant of candidateVariants(candidate)) {
      const aliased = PLUGIN_LOGO_ALIASES[variant] ?? variant;
      if (hasPluginLogoSlug(aliased)) {
        return aliased;
      }

      const compactMatch =
        imageSlugByCompactSlug.get(compactPluginSlug(aliased)) ??
        imageSlugByCompactSlug.get(variant);
      if (compactMatch) {
        return compactMatch;
      }
    }
  }

  return null;
}

export function getPluginLogoSources(
  identity: PluginVisualIdentity,
): PluginLogoSources | null {
  const slug = resolvePluginLogoSlug(identity);
  if (!slug) {
    return null;
  }

  const svg = svgLogoSlugs.has(slug)
    ? `${PLUGIN_LOGO_BASE_PATH}/svg/${slug}.svg`
    : undefined;
  const avif = avifLogoSlugs.has(slug)
    ? `${PLUGIN_LOGO_BASE_PATH}/avif/${slug}.avif`
    : undefined;
  const src = svg ?? avif ?? LEGACY_PLUGIN_LOGO_SRC_BY_SLUG[slug];

  return src ? { slug, svg, avif, src } : null;
}

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

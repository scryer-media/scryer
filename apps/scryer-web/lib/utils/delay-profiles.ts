import type { DelayProfileDraft, ParsedDelayProfile } from "@/lib/types/delay-profiles";

export const DELAY_PROFILE_CATALOG_KEY = "acquisition.delay_profiles";

const FACET_OPTIONS = ["movie", "tv", "anime"] as const;

export function buildDelayProfileTemplate(existing: ParsedDelayProfile[]): DelayProfileDraft {
  const maxPriority = existing.reduce((max, p) => Math.max(max, p.priority), 0);
  return {
    id: "",
    name: "",
    usenet_delay_minutes: 0,
    torrent_delay_minutes: 0,
    preferred_protocol: "usenet",
    min_age_minutes: 0,
    bypass_score_threshold: null,
    applies_to_facets: [],
    tags: [],
    priority: maxPriority + 1,
    enabled: true,
  };
}

export function createDelayProfileId(name: string, existing: ParsedDelayProfile[]): string {
  const base = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
  const slug = base || "profile";
  const ids = new Set(existing.map((p) => p.id));
  if (!ids.has(slug)) return slug;
  for (let i = 2; ; i++) {
    const candidate = `${slug}-${i}`;
    if (!ids.has(candidate)) return candidate;
  }
}

export function parseDelayProfileCatalog(json: string): ParsedDelayProfile[] {
  try {
    const raw = JSON.parse(json);
    if (!Array.isArray(raw)) return [];
    return raw.map(normalizeDelayProfile);
  } catch {
    return [];
  }
}

function normalizeDelayProfile(raw: Record<string, unknown>): ParsedDelayProfile {
  return {
    id: String(raw.id ?? ""),
    name: String(raw.name ?? ""),
    usenet_delay_minutes:
      typeof raw.usenet_delay_minutes === "number" ? raw.usenet_delay_minutes : 0,
    torrent_delay_minutes:
      typeof raw.torrent_delay_minutes === "number" ? raw.torrent_delay_minutes : 0,
    preferred_protocol:
      raw.preferred_protocol === "torrent" ? "torrent" : "usenet",
    min_age_minutes:
      typeof raw.min_age_minutes === "number" ? raw.min_age_minutes : 0,
    bypass_score_threshold:
      typeof raw.bypass_score_threshold === "number" ? raw.bypass_score_threshold : null,
    applies_to_facets: Array.isArray(raw.applies_to_facets)
      ? raw.applies_to_facets.filter((v): v is string => typeof v === "string")
      : [],
    tags: Array.isArray(raw.tags)
      ? raw.tags.filter((v): v is string => typeof v === "string")
      : [],
    priority: typeof raw.priority === "number" ? raw.priority : 0,
    enabled: typeof raw.enabled === "boolean" ? raw.enabled : true,
  };
}

export function serializeDelayProfileCatalog(profiles: ParsedDelayProfile[]): string {
  return JSON.stringify(profiles);
}

export { FACET_OPTIONS };

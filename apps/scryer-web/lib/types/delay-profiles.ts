export type DelayProfileProtocol = "USENET" | "TORRENT";

export type DelayProfileFacet = "MOVIE" | "SERIES" | "ANIME";

export type DelayProfileDraft = {
  id: string;
  name: string;
  /** Delay for usenet releases (minutes). 0 = grab immediately. */
  usenet_delay_minutes: number;
  /** Delay for torrent releases (minutes). 0 = grab immediately. */
  torrent_delay_minutes: number;
  /** Whether Usenet releases are eligible for this profile. */
  enable_usenet: boolean;
  /** Whether torrent releases are eligible for this profile. */
  enable_torrent: boolean;
  /** Preferred protocol — bypass eligibility only applies to preferred. */
  preferred_protocol: DelayProfileProtocol;
  /** Usenet minimum age in minutes. Hard gate, no bypass. 0 = disabled. */
  min_age_minutes: number;
  bypass_score_threshold: number | null;
  /** Whether the highest-quality eligible release bypasses its delay. */
  bypass_if_highest_quality: boolean;
  applies_to_facets: DelayProfileFacet[];
  tags: string[];
  priority: number;
  enabled: boolean;
};

export type ParsedDelayProfile = DelayProfileDraft;

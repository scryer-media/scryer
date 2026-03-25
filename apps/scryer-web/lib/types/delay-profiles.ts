export type DelayProfileProtocol = "usenet" | "torrent";

export type DelayProfileFacet = "movie" | "series" | "anime";

export type DelayProfileDraft = {
  id: string;
  name: string;
  /** Delay for usenet releases (minutes). 0 = grab immediately. */
  usenet_delay_minutes: number;
  /** Delay for torrent releases (minutes). 0 = grab immediately. */
  torrent_delay_minutes: number;
  /** Preferred protocol — score bypass only applies to preferred. */
  preferred_protocol: DelayProfileProtocol;
  /** Usenet minimum age in minutes. Hard gate, no bypass. 0 = disabled. */
  min_age_minutes: number;
  bypass_score_threshold: number | null;
  applies_to_facets: DelayProfileFacet[];
  tags: string[];
  priority: number;
  enabled: boolean;
};

export type ParsedDelayProfile = DelayProfileDraft;

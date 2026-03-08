export type DelayProfileDraft = {
  id: string;
  name: string;
  delay_hours: number;
  bypass_score_threshold: number | null;
  applies_to_facets: string[];
  tags: string[];
  priority: number;
  enabled: boolean;
};

export type ParsedDelayProfile = DelayProfileDraft;

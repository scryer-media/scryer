export type RuleSetRecord = {
  id: string;
  name: string;
  description: string;
  regoSource: string;
  enabled: boolean;
  priority: number;
  appliedFacets: string[];
  createdAt: string;
  updatedAt: string;
};

export type RuleSetDraft = {
  name: string;
  description: string;
  regoSource: string;
  enabled: boolean;
  priority: number;
  appliedFacets: string[];
};

export type RuleValidationResult = {
  valid: boolean;
  errors: string[];
};

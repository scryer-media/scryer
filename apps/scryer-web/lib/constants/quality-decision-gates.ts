import {
  CircleHelp,
  Languages,
  Layers,
  ListPlus,
  RefreshCw,
  type LucideIcon,
} from "lucide-react";

/** Gate values this web build knows, as the GraphQL enum names them. */
export type KnownQualityDecisionGate =
  "QUALITY_TIER" | "REVISION" | "PREFERRED_LANGUAGE" | "OPTIONAL_LANGUAGES";

/**
 * A step of a quality profile's decision order. A newer server may send a
 * value this build does not know; it is kept as-is.
 */
export type QualityDecisionGate = KnownQualityDecisionGate | (string & {});

/**
 * What a gate compares. Notices about the order are driven by kind, so a
 * new gate of an existing kind gets them without further code.
 */
export type QualityDecisionGateKind = "tier" | "revision" | "language";

export type QualityDecisionGateDescriptor = {
  id: KnownQualityDecisionGate;
  kind: QualityDecisionGateKind;
  labelKey: string;
  descriptionKey: string;
  icon: LucideIcon;
};

/** Every known gate, in the order the "available steps" tray lists them. */
export const QUALITY_DECISION_GATES: readonly QualityDecisionGateDescriptor[] =
  [
    {
      id: "QUALITY_TIER",
      kind: "tier",
      labelKey: "qualityProfile.decisionGateQualityTier",
      descriptionKey: "qualityProfile.decisionGateQualityTierDescription",
      icon: Layers,
    },
    {
      id: "REVISION",
      kind: "revision",
      labelKey: "qualityProfile.decisionGateRevision",
      descriptionKey: "qualityProfile.decisionGateRevisionDescription",
      icon: RefreshCw,
    },
    {
      id: "PREFERRED_LANGUAGE",
      kind: "language",
      labelKey: "qualityProfile.decisionGatePreferredLanguage",
      descriptionKey: "qualityProfile.decisionGatePreferredLanguageDescription",
      icon: Languages,
    },
    {
      id: "OPTIONAL_LANGUAGES",
      kind: "language",
      labelKey: "qualityProfile.decisionGateOptionalLanguages",
      descriptionKey: "qualityProfile.decisionGateOptionalLanguagesDescription",
      icon: ListPlus,
    },
  ];

/** Icon for a gate value this build does not know. */
export const UNKNOWN_QUALITY_DECISION_GATE_ICON: LucideIcon = CircleHelp;

const DESCRIPTOR_BY_ID: ReadonlyMap<string, QualityDecisionGateDescriptor> =
  new Map(QUALITY_DECISION_GATES.map((gate) => [gate.id, gate]));

export function qualityDecisionGateDescriptor(
  gate: string,
): QualityDecisionGateDescriptor | undefined {
  return DESCRIPTOR_BY_ID.get(gate);
}

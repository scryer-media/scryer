import type { RegistryPluginRecord } from "../../components/views/settings/settings-plugins-section.tsx";
import type {
  TrackedRulePackMember,
  TrackedRulePackRecord,
} from "../../components/views/settings/tracked-rule-packs-section.tsx";

/**
 * A plugin, or a set of alternatives, that setup suggests before the full
 * list. `reasonKey` explains who it is for; without one, the plugin's own
 * description stands in.
 */
export type SetupPluginRecommendation = {
  key: string;
  titleKey: string;
  reasonKey: string | null;
  pluginIds: string[];
};

export const SETUP_PLUGIN_RECOMMENDATIONS: SetupPluginRecommendation[] = [
  {
    key: "archive-extraction",
    titleKey: "setup.recommendedArchiveTitle",
    reasonKey: "setup.recommendedArchiveReason",
    pluginIds: ["archive-extraction"],
  },
  {
    key: "qbittorrent",
    titleKey: "setup.recommendedQbittorrentTitle",
    reasonKey: null,
    pluginIds: ["qbittorrent"],
  },
  {
    key: "subtitles",
    titleKey: "setup.recommendedSubtitlesTitle",
    reasonKey: "setup.recommendedSubtitlesReason",
    pluginIds: [
      "enhanced-subtitle-sync",
      "opensubtitles",
      "jimaku",
      "animetosho-xyz-subtitles",
    ],
  },
  {
    key: "advanced-indexers",
    titleKey: "setup.recommendedAdvancedIndexersTitle",
    reasonKey: "setup.recommendedAdvancedIndexersReason",
    pluginIds: ["cardigann-engine"],
  },
  {
    key: "media-server",
    titleKey: "setup.recommendedMediaServerTitle",
    reasonKey: "setup.recommendedMediaServerReason",
    pluginIds: ["jellyfin", "emby", "plex"],
  },
];

export type ResolvedSetupPluginRecommendation = SetupPluginRecommendation & {
  plugins: RegistryPluginRecord[];
};

/** The plugins setup lists: official ones that do not ship with Scryer. */
function listedPlugins(plugins: RegistryPluginRecord[]) {
  return plugins.filter((plugin) => plugin.official && !plugin.builtin);
}

/**
 * The recommendations whose plugins the registry offers, in recommendation
 * order. A recommendation with none of its plugins available is left out.
 */
export function resolveSetupPluginRecommendations(
  plugins: RegistryPluginRecord[],
): ResolvedSetupPluginRecommendation[] {
  const byId = new Map(listedPlugins(plugins).map((plugin) => [plugin.id, plugin]));
  return SETUP_PLUGIN_RECOMMENDATIONS.flatMap((recommendation) => {
    const resolved = recommendation.pluginIds.flatMap((id) => {
      const plugin = byId.get(id);
      return plugin ? [plugin] : [];
    });
    return resolved.length > 0 ? [{ ...recommendation, plugins: resolved }] : [];
  });
}

/** The listed plugins that no recommendation already shows. */
export function otherSetupPlugins(plugins: RegistryPluginRecord[]) {
  const recommended = new Set(
    SETUP_PLUGIN_RECOMMENDATIONS.flatMap((recommendation) => recommendation.pluginIds),
  );
  return listedPlugins(plugins).filter((plugin) => !recommended.has(plugin.id));
}

export const TRASH_RULE_PACK_ID = "trash-guides-scoring-pack";
export const SEADEX_RULE_PACK_ID = "seadex-scoring-pack";

export type SetupRulePackRecommendation = {
  packId: string;
  titleKey: string;
  reasonKey: string;
};

export const SETUP_RULE_PACK_RECOMMENDATIONS: SetupRulePackRecommendation[] = [
  {
    packId: TRASH_RULE_PACK_ID,
    titleKey: "setup.rulePackTrashTitle",
    reasonKey: "setup.rulePackTrashReason",
  },
  {
    packId: SEADEX_RULE_PACK_ID,
    titleKey: "setup.rulePackSeadexTitle",
    reasonKey: "setup.rulePackSeadexReason",
  },
];

function activeMembers(pack: TrackedRulePackRecord): TrackedRulePackMember[] {
  return pack.members.filter((member) => !member.removed);
}

/** Whether an installed pack scores anything: any of its rules is on. */
export function rulePackEnabled(pack: TrackedRulePackRecord | undefined) {
  return pack ? activeMembers(pack).some((member) => member.enabled === true) : false;
}

export function enabledRulePackTemplateIds(pack: TrackedRulePackRecord) {
  return activeMembers(pack)
    .filter((member) => member.enabled === true)
    .map((member) => member.templateId);
}

/**
 * The rules to switch on when a pack is turned on from setup: the ones its
 * author enables by default, or every rule when the pack marks none (SeaDex
 * is a single rule).
 */
export function defaultRulePackTemplateIds(
  templates: Array<{ id: string; defaultEnabled: boolean }>,
) {
  const defaults = templates.filter((template) => template.defaultEnabled);
  return (defaults.length > 0 ? defaults : templates).map((template) => template.id);
}

/**
 * Settings for an installed pack with exactly `enabledTemplateIds` on, keeping
 * its priorities and auto-update choice.
 */
export function rulePackSettingsInput(
  pack: TrackedRulePackRecord,
  enabledTemplateIds: string[],
) {
  const members = activeMembers(pack);
  const wanted = new Set(enabledTemplateIds);
  return {
    packId: pack.packId,
    enabledTemplateIds: members
      .filter((member) => wanted.has(member.templateId))
      .map((member) => member.templateId),
    priorities: members.flatMap((member) =>
      member.priority !== null
        ? [{ templateId: member.templateId, priority: member.priority }]
        : [],
    ),
    autoUpdate: pack.autoUpdate,
    expectedRevision: pack.revision,
  };
}

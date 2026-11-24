import type { TourDefinition, TourGate } from "./types.ts";

// Sidebar stops have nothing to point at on a phone, where the sidebar is
// folded away, so they are left out there.
const onDesktop: TourGate = ({ isMobile }) => !isMobile;

/**
 * Every guided tour the web offers. Add a tour or a step here; the engine,
 * persistence and gating need no other change.
 *
 * - `onboarding` is offered once to a new account.
 * - Each `highlights` tour introduces one release. An account sees the newest
 *   one it has not seen; finishing it settles the older ones too.
 * - A shipped tour id is permanent: it is stored with every account that saw
 *   it. A materially new tour gets a new id.
 * - Steps point at durable `selectorId` ids only, and every key they name
 *   must exist in the English dictionary (the registry test checks both).
 * - A step's gate matches the gate of what it points at, so a viewer is never
 *   shown a stop they cannot open.
 */
export const TOUR_REGISTRY: readonly TourDefinition[] = [
  {
    id: "onboarding",
    kind: "onboarding",
    steps: [
      {
        id: "welcome",
        titleKey: "tours.onboarding.welcome.title",
        bodyKey: "tours.onboarding.welcome.body",
        icon: "sparkles",
      },
      {
        id: "search",
        titleKey: "tours.onboarding.search.title",
        bodyKey: "tours.onboarding.search.body",
        icon: "search",
        target: ["global-search-trigger"],
        mobileTarget: ["global-search-mobile-trigger"],
      },
      {
        id: "libraries",
        titleKey: "tours.onboarding.libraries.title",
        bodyKey: "tours.onboarding.libraries.body",
        icon: "library",
        target: ["root-sidebar-group", "catalogs"],
        when: (context) =>
          onDesktop(context) && context.permissions.canViewCatalog,
      },
      {
        id: "discover",
        titleKey: "tours.onboarding.discover.title",
        bodyKey: "tours.onboarding.discover.body",
        icon: "discover",
        target: ["root-sidebar-group", "discover"],
        when: (context) =>
          onDesktop(context) &&
          (context.permissions.canRequestMedia ||
            context.permissions.canManageTitle),
      },
      {
        id: "lists",
        titleKey: "tours.onboarding.lists.title",
        bodyKey: "tours.onboarding.lists.body",
        icon: "lists",
        target: ["root-sidebar-nav", "lists"],
        when: (context) =>
          onDesktop(context) && context.permissions.canAccessLists,
      },
      {
        id: "activity",
        titleKey: "tours.onboarding.activity.title",
        bodyKey: "tours.onboarding.activity.body",
        icon: "activity",
        target: ["root-sidebar-nav", "activity"],
        when: (context) =>
          onDesktop(context) && context.permissions.canAccessActivity,
      },
      {
        id: "indexers",
        titleKey: "tours.onboarding.indexers.title",
        bodyKey: "tours.onboarding.indexers.body",
        icon: "system",
        target: ["root-sidebar-settings-shortcut", "indexers"],
        action: {
          labelKey: "tours.onboarding.indexers.action",
          route: "/integrations/indexers",
        },
        when: (context) =>
          onDesktop(context) && context.permissions.canManageSystemSettings,
      },
      {
        id: "media-servers",
        titleKey: "tours.onboarding.mediaServers.title",
        bodyKey: "tours.onboarding.mediaServers.body",
        icon: "library",
        target: ["root-sidebar-settings-shortcut", "mediaservers"],
        when: (context) =>
          onDesktop(context) && context.permissions.canManageSystemSettings,
      },
      {
        id: "quality-profiles",
        titleKey: "tours.onboarding.qualityProfiles.title",
        bodyKey: "tours.onboarding.qualityProfiles.body",
        icon: "settings",
        route: "/settings/quality-profiles",
        via: ["root-sidebar-nav", "settings"],
        target: ["settings-quality-profiles-table-card"],
        when: (context) => context.permissions.canManageCatalogSettings,
      },
      {
        id: "rules",
        titleKey: "tours.onboarding.rules.title",
        bodyKey: "tours.onboarding.rules.body",
        icon: "rules",
        target: ["root-sidebar-settings-shortcut", "rules"],
        when: (context) =>
          onDesktop(context) && context.permissions.canManageCatalogSettings,
      },
      {
        id: "notifications",
        titleKey: "tours.onboarding.notifications.title",
        bodyKey: "tours.onboarding.notifications.body",
        icon: "notifications",
        route: "/integrations/notifications",
        via: ["root-sidebar-settings-shortcut", "notifications"],
        // The notification plugins tray: docked beside the page on a wide
        // screen, behind its button on a narrower one.
        target: ["settings-plugins-panel"],
        fallbackTarget: ["settings-plugins-panel-toggle"],
        when: (context) => context.permissions.canManageSystemSettings,
      },
      {
        id: "plugins",
        titleKey: "tours.onboarding.plugins.title",
        bodyKey: "tours.onboarding.plugins.body",
        icon: "plugins",
        route: "/settings/plugins",
        via: ["root-sidebar-nav", "settings"],
        target: ["settings-plugins-installed-table"],
        when: (context) => context.permissions.canManageSystemSettings,
      },
    ],
  },
];

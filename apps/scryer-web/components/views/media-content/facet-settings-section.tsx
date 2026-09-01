import type { ReactNode } from "react";
import { Link } from "react-router";
import {
  ChevronRight,
  FileType,
  Library,
  Route,
  Settings2,
  SlidersHorizontal,
  type LucideIcon,
} from "lucide-react";
import type { ViewId } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";
import { buildViewPath } from "@/lib/utils/routing";

export type FacetSettingsSectionId =
  | "library"
  | "general"
  | "quality"
  | "renaming"
  | "routing";

const SECTION_ORDER: FacetSettingsSectionId[] = [
  "library",
  "general",
  "quality",
  "renaming",
  "routing",
];

const SECTION_META: Record<
  FacetSettingsSectionId,
  { labelKey: string; icon: LucideIcon }
> = {
  library: { labelKey: "nav.library", icon: Library },
  general: { labelKey: "facetSettings.general", icon: Settings2 },
  quality: { labelKey: "facetSettings.quality", icon: SlidersHorizontal },
  renaming: { labelKey: "facetSettings.renaming", icon: FileType },
  routing: { labelKey: "facetSettings.routing", icon: Route },
};

/** DOM id of the optional secondary-nav column. The per-library list gutter is
 * portaled into this slot so it stacks beside the section sub-nav. */
export const LIBRARY_SECONDARY_NAV_SLOT_ID = "facet-library-secondary-nav";
/** DOM id of the header-actions slot. The per-library scan button is portaled here
 * so it sits on the same row as the page header. */
export const LIBRARY_HEADER_ACTIONS_SLOT_ID = "facet-library-header-actions";
/** DOM id of the pane footer slot. The per-library save bar is portaled here so it
 * pins to the bottom of the content pane instead of floating after short content. */
export const LIBRARY_FOOTER_SLOT_ID = "facet-library-footer";
/** DOM id of the right reference rail for settings pages with contextual help. */
export const FACET_REFERENCE_SLOT_ID = "facet-settings-reference";

type FacetSettingsSectionProps = {
  /** The active facet view (movies/series/anime), used to build sub-page links. */
  view: ViewId;
  /** The active facet settings sub-page. */
  section: FacetSettingsSectionId;
  /** The facet label shown in the breadcrumb and subnav header, e.g. "Movies". */
  facetLabel: string;
  /** Whether the user can manage full catalog config (all sub-pages). */
  canManageConfig: boolean;
  /** Whether the user can manage library settings (library sub-page only). */
  canManageLibrarySettings: boolean;
  /** The existing settings form/panel for this section. */
  children: ReactNode;
  /** Optional status node rendered at the right of the page header (e.g. a live scan pill). */
  headerStatus?: ReactNode;
  /** When true, render an empty secondary-nav column (portal target) beside the sub-nav. */
  showSecondaryNav?: boolean;
  /** Content column width. "full" suits the widest tables; "reference" keeps a form column plus side reference. */
  contentWidth?: "default" | "wide" | "full" | "reference";
  /** Optional trailing breadcrumb segment (e.g. the active library name). */
  trailingCrumb?: string;
};

/**
 * Wraps the existing facet settings forms in the same page framing as the
 * global Settings area (Settings > Quality Profiles): an optional left subnav
 * for switching sub-pages, a breadcrumb, an icon-tile page header, and a
 * centered, scrollable content column. The forms themselves are passed through
 * unchanged as `children`.
 */
export function FacetSettingsSection({
  view,
  section,
  facetLabel,
  canManageConfig,
  canManageLibrarySettings,
  children,
  headerStatus,
  showSecondaryNav = false,
  contentWidth = "default",
  trailingCrumb,
}: FacetSettingsSectionProps) {
  const t = useTranslate();
  const meta = SECTION_META[section];
  const sectionLabel = t(meta.labelKey);
  const Icon = meta.icon;

  // Mirror the root sidebar's visibleMediaSettingsSubPages permission rule.
  const availableSections = canManageConfig
    ? SECTION_ORDER
    : canManageLibrarySettings
      ? SECTION_ORDER.filter((id) => id === "library")
      : [];
  const showSubnav = availableSections.length > 1;
  const pageContent = (
    <>
      <div className="mb-4 flex items-center gap-1.5 text-[12.5px] text-[var(--scry-faint)]">
        <span>{facetLabel}</span>
        <ChevronRight className="h-3.5 w-3.5" />
        <span>{t("nav.settings")}</span>
        <ChevronRight className="h-3.5 w-3.5" />
        <span
          className={cn(
            "font-semibold",
            trailingCrumb
              ? "text-[var(--scry-muted)]"
              : "text-[var(--scry-accent-text)]",
          )}
        >
          {sectionLabel}
        </span>
        {trailingCrumb ? (
          <>
            <ChevronRight className="h-3.5 w-3.5" />
            <span className="font-semibold text-[var(--scry-accent-text)]">
              {trailingCrumb}
            </span>
          </>
        ) : null}
      </div>
      <div className="mb-6 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex min-w-0 items-center gap-4">
          <div className="flex h-[46px] w-[46px] shrink-0 items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[linear-gradient(135deg,rgba(var(--scry-accent-rgb),0.35),rgba(123,91,255,0.22))] text-[var(--scry-accent-text)]">
            <Icon className="h-[23px] w-[23px]" />
          </div>
          <div className="min-w-0">
            <h1 className="text-[25px] font-bold leading-none tracking-normal text-[var(--scry-ink2)]">
              {sectionLabel}
            </h1>
          </div>
        </div>
        {headerStatus || showSecondaryNav ? (
          <div className="flex shrink-0 items-center gap-2.5">
            {headerStatus}
            {showSecondaryNav ? (
              <div
                id={LIBRARY_HEADER_ACTIONS_SLOT_ID}
                className="flex items-center gap-2.5"
              />
            ) : null}
          </div>
        ) : null}
      </div>
      {children}
    </>
  );

  return (
    <div className="flex min-h-0 w-full flex-1 flex-col overflow-hidden bg-transparent md:flex-row">
      {showSubnav ? (
        <aside
          data-slot="facet-settings-subnav"
          className="w-full shrink-0 border-b border-[var(--scry-border3)] bg-[var(--scry-surfF)] p-3 md:h-full md:w-[218px] md:overflow-y-auto md:border-b-0 md:border-r md:p-[22px_14px]"
        >
          <div className="mb-3 flex items-center gap-2 px-2 text-[var(--scry-ink2)] md:mb-4">
            <Settings2 className="h-[18px] w-[18px] text-[var(--scry-accent-text)]" />
            <span className="text-[16px] font-bold">{facetLabel}</span>
          </div>
          <nav className="flex gap-2 overflow-x-auto pb-1 md:flex-col md:overflow-visible md:pb-0">
            {availableSections.map((id) => {
              const ItemIcon = SECTION_META[id].icon;
              const active = section === id;
              return (
                <Link
                  key={id}
                  id={selectorId("root-sidebar-media", view, id)}
                  to={buildViewPath(view, undefined, id)}
                  className={cn(
                    "flex h-9 shrink-0 items-center gap-2 rounded-[9px] px-3 text-[13px] font-medium text-[var(--scry-muted)] transition hover:bg-[var(--scry-hover)] hover:text-[var(--scry-ink2)] md:w-full",
                    active &&
                      "bg-[linear-gradient(90deg,rgba(var(--scry-accent-rgb),0.26),rgba(var(--scry-accent-rgb),0.08))] text-[var(--scry-ink2)] shadow-[inset_2px_0_0_var(--scry-accent-ring)]",
                  )}
                >
                  <ItemIcon
                    className={cn(
                      "h-[17px] w-[17px] text-[var(--scry-muted2)]",
                      active && "text-[var(--scry-accent-text)]",
                    )}
                  />
                  <span className="whitespace-nowrap">
                    {t(SECTION_META[id].labelKey)}
                  </span>
                </Link>
              );
            })}
          </nav>
        </aside>
      ) : null}
      {showSecondaryNav ? (
        <aside
          id={LIBRARY_SECONDARY_NAV_SLOT_ID}
          data-slot="facet-secondary-nav"
          className="w-full shrink-0 border-b border-[var(--scry-border3)] bg-[var(--scry-surfF)] p-3 md:h-full md:w-[220px] md:overflow-y-auto md:border-b-0 md:border-r md:p-[22px_14px]"
        />
      ) : null}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-transparent">
        <div
          data-slot="facet-settings-scroll"
          className="min-h-0 flex-1 overflow-y-auto"
        >
        <div
          className={cn(
            "mx-auto w-full px-4 py-5 sm:px-6 md:px-[30px] md:py-[26px] md:pb-[60px]",
            contentWidth === "reference"
              ? "max-w-[1780px]"
              : contentWidth === "full"
                ? "max-w-[1780px]"
                : contentWidth === "wide"
                  ? "max-w-[1280px]"
                  : "max-w-[920px]",
          )}
        >
          {contentWidth === "reference" ? (
            <div className="grid gap-5 xl:grid-cols-[minmax(0,920px)_minmax(28rem,1fr)] 2xl:grid-cols-[minmax(0,920px)_minmax(42rem,1fr)] xl:items-start">
              <div className="min-w-0">{pageContent}</div>
              <aside
                id={FACET_REFERENCE_SLOT_ID}
                data-slot="facet-reference"
                className="min-w-0 xl:sticky xl:top-[26px]"
              />
            </div>
          ) : (
            pageContent
          )}
        </div>
        </div>
        {showSecondaryNav ? (
          <div id={LIBRARY_FOOTER_SLOT_ID} data-slot="facet-footer" />
        ) : null}
      </div>
    </div>
  );
}

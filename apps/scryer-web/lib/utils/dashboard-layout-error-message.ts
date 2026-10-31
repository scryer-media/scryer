import type { Translate } from "@/components/root/types";
import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

/**
 * The server names why it refused a dashboard layout as `extensions.reason` on
 * a `VALIDATION_ERROR`. The grid never builds a layout that trips these, so
 * they surface only if a saved layout and this build disagree; each known
 * reason is said in the viewer's language and anything else falls back to the
 * server's sentence.
 */
export const DASHBOARD_LAYOUT_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  SETTINGS_DASHBOARD_LAYOUT_COLUMNS_OUT_OF_RANGE:
    "dashboard.layout.refusal.columnsOutOfRange",
  SETTINGS_DASHBOARD_LAYOUT_TOO_MANY_ITEMS:
    "dashboard.layout.refusal.tooManyPanels",
  SETTINGS_DASHBOARD_PANEL_ID_INVALID:
    "dashboard.layout.refusal.panelIdInvalid",
  SETTINGS_DASHBOARD_PANEL_ID_DUPLICATED:
    "dashboard.layout.refusal.panelIdDuplicate",
  // A panel off the top or left edge and one past the right edge both mean it
  // is outside the grid.
  SETTINGS_DASHBOARD_ITEM_POSITION_OUT_OF_RANGE:
    "dashboard.layout.refusal.outsideGrid",
  SETTINGS_DASHBOARD_ITEM_EXCEEDS_COLUMNS:
    "dashboard.layout.refusal.outsideGrid",
  SETTINGS_DASHBOARD_ITEM_SIZE_OUT_OF_RANGE:
    "dashboard.layout.refusal.sizeInvalid",
};

/** The translation key for the reason a layout was refused, if known. */
export function dashboardLayoutRefusalKey(error: unknown): string | null {
  const reason = graphQlErrorExtension(error, "VALIDATION_ERROR", "reason");
  return reason !== null && Object.hasOwn(DASHBOARD_LAYOUT_REFUSAL_KEYS, reason)
    ? DASHBOARD_LAYOUT_REFUSAL_KEYS[reason]
    : null;
}

/**
 * What to show for a failed layout save or reset: the translated reason when
 * the server named one this client knows, otherwise the server's sentence, and
 * `fallback` when there is no usable sentence.
 */
export function dashboardLayoutErrorMessage(
  error: unknown,
  t: Translate,
  fallback: string,
): string {
  const key = dashboardLayoutRefusalKey(error);
  return key ? t(key) : userFacingGraphQlErrorMessage(error, fallback, t);
}

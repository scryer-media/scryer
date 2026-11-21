import type { Translate } from "@/components/root/types";

/**
 * Blocklist reasons Scryer stores as a code rather than a sentence. Each is
 * said in the viewer's language; any other reason is shown as stored.
 */
export const BLOCKLIST_REASON_KEYS: Readonly<Record<string, string>> = {
  BLOCKLIST_IMPORT_NOT_AN_UPGRADE:
    "error.reason.BLOCKLIST_IMPORT_NOT_AN_UPGRADE",
};

/** A blocklist entry's reason, translated when it is a known code. */
export function blocklistReasonText(reason: string, t: Translate): string {
  return Object.hasOwn(BLOCKLIST_REASON_KEYS, reason)
    ? t(BLOCKLIST_REASON_KEYS[reason])
    : reason;
}

/** Conditions that require media review, independent of server wording. */
export const MEDIA_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  MEDIA_UNVERIFIED_REPLACEMENT: "error.reason.MEDIA_UNVERIFIED_REPLACEMENT",
  MEDIA_REFERENCES_REQUIRE_REVIEW:
    "error.reason.MEDIA_REFERENCES_REQUIRE_REVIEW",
  MEDIA_PROBE_IO_FAILED: "error.reason.MEDIA_PROBE_IO_FAILED",
  MEDIA_PROBE_MALFORMED: "error.reason.MEDIA_PROBE_MALFORMED",
  MEDIA_PROBE_ENCRYPTED: "error.reason.MEDIA_PROBE_ENCRYPTED",
};

-- Project an import rejection's skip reason onto its domain event row.
--
-- History lists a post-download rule's refusal under its own type, apart from
-- failed and skipped imports. The skip reason lives only in the compressed
-- payload, so filtering on it needs a plain column, like `import_status`.
-- Rows stored before this column existed are filled by the migration's Rust
-- step, which decodes each import rejection's payload.

ALTER TABLE domain_events ADD COLUMN IF NOT EXISTS import_skip_reason TEXT;

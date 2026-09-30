-- The entity kind of each id a list exclusion matches on (`movie`, `tv`, ...).
-- One provider hands out the same number for unrelated entities, so an
-- exclusion that knows its id's kind must not cover an item whose id names a
-- different kind. Rows written before this migration carry no kind ('') and
-- keep matching any kind, as they always have.
ALTER TABLE list_exclusion_external_ids ADD COLUMN external_kind text NOT NULL DEFAULT '';

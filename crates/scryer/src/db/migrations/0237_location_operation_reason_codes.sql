-- Machine-readable reason codes on location operations and their title
-- checkpoints (location failure recovery, 0.20.0).
--
-- `failure_reason` keeps the raw cause verbatim; `reason_code` is what the
-- Activity panel keys its guidance on (retry the same operation, plan again,
-- or fix the storage first). Nullable: rows written before this migration, and
-- rows that finished cleanly, carry no code. Values are the snake_case names of
-- `LocationReasonCode` in scryer-application::location::model.
ALTER TABLE location_operations ADD COLUMN reason_code TEXT;
ALTER TABLE location_operation_title_checkpoints ADD COLUMN reason_code TEXT;

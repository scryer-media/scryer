-- Give each title folder a comparison key, and room to record why a title's
-- folder was cleared.
--
-- `folder_path_key` is the folder as it is compared: trailing separators and
-- `.` parts dropped, Unicode composed, and case folded only for Windows paths.
-- Two titles whose folders have the same key share one folder.
--
-- When a title's folder is cleared because it collided with another title's,
-- `folder_conflict_reason` holds the reason code and `folder_conflict_path`
-- the folder it lost. Both are cleared when the title records a folder again.
ALTER TABLE titles ADD COLUMN folder_path_key TEXT;
ALTER TABLE titles ADD COLUMN folder_conflict_reason TEXT;
ALTER TABLE titles ADD COLUMN folder_conflict_path TEXT;

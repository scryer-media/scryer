-- Set when a location move found its destination already there and the
-- quick check, which keeps no hashes, proved it the source's copy. A record
-- with no hashes and this unset placed the source's own file (a rename or a
-- second link); every record written before this column existed is one.
ALTER TABLE location_operation_verifications
    ADD COLUMN content_matched boolean DEFAULT false NOT NULL;

-- The stored mark of the file a media file's full hash was read of, written
-- and cleared in the same statement as full_blake3. A hash stored before
-- this column existed has none and is judged by size and modification time.
ALTER TABLE media_files ADD COLUMN full_hash_mark TEXT;

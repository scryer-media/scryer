-- A token that changes with everything the discovery affinity profile reads.
--
-- The profile summarizes the canonical tags of every readable title, so it is
-- worth keeping between requests only while none of its inputs has changed.
-- Each write to those inputs replaces the token inside its own transaction (one
-- statement-level trigger per write statement),
-- so a reader that sees an unchanged token is reading the same inputs. The
-- token is random rather than a counter: a restored database can never
-- present a token an earlier state of this process already used. A restore
-- clears the row with the rest of Discovery's state; the next write to an
-- input creates it again, and until then affinity is simply not cached.
CREATE TABLE IF NOT EXISTS discovery_affinity_revision (
    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
    revision TEXT NOT NULL
);

INSERT INTO discovery_affinity_revision (id, revision)
VALUES (1, md5(random()::text || clock_timestamp()::text))
ON CONFLICT (id) DO NOTHING;

CREATE OR REPLACE FUNCTION discovery_affinity_bump_revision()
RETURNS TRIGGER AS $$
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, md5(random()::text || clock_timestamp()::text))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS titles_insert_bump_affinity_revision ON titles;
CREATE TRIGGER titles_insert_bump_affinity_revision
AFTER INSERT ON titles
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS titles_delete_bump_affinity_revision ON titles;
CREATE TRIGGER titles_delete_bump_affinity_revision
AFTER DELETE ON titles
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS titles_update_bump_affinity_revision ON titles;
CREATE TRIGGER titles_update_bump_affinity_revision
AFTER UPDATE OF library_id, facet ON titles
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS title_metadata_tags_insert_bump_affinity_revision ON title_metadata_tags;
CREATE TRIGGER title_metadata_tags_insert_bump_affinity_revision
AFTER INSERT ON title_metadata_tags
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS title_metadata_tags_delete_bump_affinity_revision ON title_metadata_tags;
CREATE TRIGGER title_metadata_tags_delete_bump_affinity_revision
AFTER DELETE ON title_metadata_tags
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS title_metadata_tags_update_bump_affinity_revision ON title_metadata_tags;
CREATE TRIGGER title_metadata_tags_update_bump_affinity_revision
AFTER UPDATE ON title_metadata_tags
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS title_metadata_tag_sources_insert_bump_affinity_revision ON title_metadata_tag_sources;
CREATE TRIGGER title_metadata_tag_sources_insert_bump_affinity_revision
AFTER INSERT ON title_metadata_tag_sources
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS title_metadata_tag_sources_delete_bump_affinity_revision ON title_metadata_tag_sources;
CREATE TRIGGER title_metadata_tag_sources_delete_bump_affinity_revision
AFTER DELETE ON title_metadata_tag_sources
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS title_metadata_tag_sources_update_bump_affinity_revision ON title_metadata_tag_sources;
CREATE TRIGGER title_metadata_tag_sources_update_bump_affinity_revision
AFTER UPDATE ON title_metadata_tag_sources
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS title_metadata_tag_source_keys_insert_bump_affinity_revision ON title_metadata_tag_source_keys;
CREATE TRIGGER title_metadata_tag_source_keys_insert_bump_affinity_revision
AFTER INSERT ON title_metadata_tag_source_keys
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS title_metadata_tag_source_keys_delete_bump_affinity_revision ON title_metadata_tag_source_keys;
CREATE TRIGGER title_metadata_tag_source_keys_delete_bump_affinity_revision
AFTER DELETE ON title_metadata_tag_source_keys
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS title_metadata_tag_source_keys_update_bump_affinity_revision ON title_metadata_tag_source_keys;
CREATE TRIGGER title_metadata_tag_source_keys_update_bump_affinity_revision
AFTER UPDATE ON title_metadata_tag_source_keys
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS discovery_submitted_subjects_insert_bump_affinity_revision ON discovery_submitted_subjects;
CREATE TRIGGER discovery_submitted_subjects_insert_bump_affinity_revision
AFTER INSERT ON discovery_submitted_subjects
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS discovery_submitted_subjects_delete_bump_affinity_revision ON discovery_submitted_subjects;
CREATE TRIGGER discovery_submitted_subjects_delete_bump_affinity_revision
AFTER DELETE ON discovery_submitted_subjects
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

DROP TRIGGER IF EXISTS discovery_submitted_subjects_update_bump_affinity_revision ON discovery_submitted_subjects;
CREATE TRIGGER discovery_submitted_subjects_update_bump_affinity_revision
AFTER UPDATE ON discovery_submitted_subjects
FOR EACH STATEMENT
EXECUTE FUNCTION discovery_affinity_bump_revision();

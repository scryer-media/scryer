-- A token that changes with everything the discovery affinity profile reads.
--
-- The profile summarizes the canonical tags of every readable title, so it is
-- worth keeping between requests only while none of its inputs has changed.
-- Each write to those inputs replaces the token inside its own transaction,
-- so a reader that sees an unchanged token is reading the same inputs. The
-- token is random rather than a counter: a restored database can never
-- present a token an earlier state of this process already used. A restore
-- clears the row with the rest of Discovery's state; the next write to an
-- input creates it again, and until then affinity is simply not cached.
CREATE TABLE IF NOT EXISTS discovery_affinity_revision (
    id INTEGER PRIMARY KEY NOT NULL CHECK (id = 1),
    revision TEXT NOT NULL
);

INSERT OR IGNORE INTO discovery_affinity_revision (id, revision)
VALUES (1, lower(hex(randomblob(16))));

CREATE TRIGGER IF NOT EXISTS titles_insert_bump_affinity_revision
AFTER INSERT ON titles
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS titles_delete_bump_affinity_revision
AFTER DELETE ON titles
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS titles_update_bump_affinity_revision
AFTER UPDATE OF library_id, facet ON titles
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS title_metadata_tags_insert_bump_affinity_revision
AFTER INSERT ON title_metadata_tags
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS title_metadata_tags_delete_bump_affinity_revision
AFTER DELETE ON title_metadata_tags
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS title_metadata_tags_update_bump_affinity_revision
AFTER UPDATE ON title_metadata_tags
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS title_metadata_tag_sources_insert_bump_affinity_revision
AFTER INSERT ON title_metadata_tag_sources
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS title_metadata_tag_sources_delete_bump_affinity_revision
AFTER DELETE ON title_metadata_tag_sources
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS title_metadata_tag_sources_update_bump_affinity_revision
AFTER UPDATE ON title_metadata_tag_sources
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS title_metadata_tag_source_keys_insert_bump_affinity_revision
AFTER INSERT ON title_metadata_tag_source_keys
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS title_metadata_tag_source_keys_delete_bump_affinity_revision
AFTER DELETE ON title_metadata_tag_source_keys
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS title_metadata_tag_source_keys_update_bump_affinity_revision
AFTER UPDATE ON title_metadata_tag_source_keys
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS discovery_submitted_subjects_insert_bump_affinity_revision
AFTER INSERT ON discovery_submitted_subjects
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS discovery_submitted_subjects_delete_bump_affinity_revision
AFTER DELETE ON discovery_submitted_subjects
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

CREATE TRIGGER IF NOT EXISTS discovery_submitted_subjects_update_bump_affinity_revision
AFTER UPDATE ON discovery_submitted_subjects
BEGIN
    INSERT INTO discovery_affinity_revision (id, revision)
    VALUES (1, lower(hex(randomblob(16))))
    ON CONFLICT (id) DO UPDATE SET revision = excluded.revision;
END;

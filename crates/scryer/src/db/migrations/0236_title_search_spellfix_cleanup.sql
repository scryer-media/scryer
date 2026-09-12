-- Merge retirement and title cascades can delete terms without going through
-- the projection writer. Keep the virtual-table row in the same transaction,
-- before SQLite can reuse the deleted INTEGER PRIMARY KEY for another term.
CREATE TRIGGER title_search_terms_delete_spellfix
AFTER DELETE ON title_search_terms
BEGIN
    DELETE FROM title_search_spellfix WHERE rowid = OLD.term_id;
END;

-- Repair vocabulary orphaned by earlier merges. Preserve every entry still
-- owned by a term; a full projection rebuild is unnecessary.
DELETE FROM title_search_spellfix
WHERE rowid NOT IN (SELECT term_id FROM title_search_terms);

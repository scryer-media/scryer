-- Distinct native spellings and language profiles must survive UI folding.
DROP INDEX idx_title_search_terms_title_kind_normalized;
CREATE UNIQUE INDEX idx_title_search_terms_title_kind_language
ON title_search_terms (title_id, term_kind, literal_term, COALESCE(language_tag, ''));
UPDATE title_search_meta SET collation_version = '' WHERE id = 1;

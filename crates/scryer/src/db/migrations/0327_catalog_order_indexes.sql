CREATE INDEX idx_titles_catalog_order_asc ON titles (
    catalog_sort_key COLLATE BINARY ASC,
    catalog_name_tie_key COLLATE BINARY ASC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year ASC,
    id COLLATE BINARY ASC
);
CREATE INDEX idx_titles_catalog_order_desc ON titles (
    catalog_sort_key COLLATE BINARY DESC,
    catalog_name_tie_key COLLATE BINARY DESC,
    (CASE WHEN year IS NULL THEN 1 ELSE 0 END) ASC,
    year DESC,
    id COLLATE BINARY DESC
);

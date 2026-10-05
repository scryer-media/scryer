-- Catalog table columns and view modes are remembered per device class, so a
-- user can keep a different layout on a phone than on a desktop. Existing
-- column rows predate the split and become the desktop layout.
--
-- The old table is renamed aside (with its index-backed primary key, whose
-- name is schema-wide) so the rebuilt table can keep the canonical names.
ALTER TABLE user_ui_table_columns RENAME TO user_ui_table_columns_0274;
ALTER TABLE user_ui_table_columns_0274
    RENAME CONSTRAINT user_ui_table_columns_pkey TO user_ui_table_columns_0274_pkey;
DROP INDEX IF EXISTS idx_user_ui_table_columns_user_view;

CREATE TABLE user_ui_table_columns (
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_class text DEFAULT 'desktop'::text NOT NULL,
    facet text NOT NULL,
    table_view_mode text NOT NULL,
    column_id text NOT NULL,
    column_order integer NOT NULL,
    visible boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT user_ui_table_columns_pkey
        PRIMARY KEY (user_id, device_class, facet, table_view_mode, column_id)
);

INSERT INTO user_ui_table_columns (
    user_id, device_class, facet, table_view_mode, column_id, column_order, visible,
    created_at, updated_at
)
SELECT user_id, 'desktop', facet, table_view_mode, column_id, column_order, visible,
       created_at, updated_at
  FROM user_ui_table_columns_0274;

DROP TABLE user_ui_table_columns_0274;

CREATE INDEX idx_user_ui_table_columns_user_view
    ON user_ui_table_columns USING btree (user_id, device_class, facet, table_view_mode, column_order);

CREATE TABLE user_ui_catalog_views (
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_class text NOT NULL,
    facet text NOT NULL,
    view_mode text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    PRIMARY KEY (user_id, device_class, facet)
);

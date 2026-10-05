-- Catalog table columns and view modes are remembered per device class, so a
-- user can keep a different layout on a phone than on a desktop. Existing
-- column rows predate the split and become the desktop layout.
--
-- Nothing references user_ui_table_columns, so the rebuild can drop the old
-- table directly.
CREATE TABLE user_ui_table_columns_0273 (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_class TEXT NOT NULL DEFAULT 'desktop',
    facet TEXT NOT NULL,
    table_view_mode TEXT NOT NULL,
    column_id TEXT NOT NULL,
    column_order INTEGER NOT NULL,
    visible INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, device_class, facet, table_view_mode, column_id)
);

INSERT INTO user_ui_table_columns_0273 (
    user_id, device_class, facet, table_view_mode, column_id, column_order, visible,
    created_at, updated_at
)
SELECT user_id, 'desktop', facet, table_view_mode, column_id, column_order, visible,
       created_at, updated_at
  FROM user_ui_table_columns;

DROP TABLE user_ui_table_columns;

ALTER TABLE user_ui_table_columns_0273 RENAME TO user_ui_table_columns;

CREATE INDEX idx_user_ui_table_columns_user_view
    ON user_ui_table_columns(user_id, device_class, facet, table_view_mode, column_order);

CREATE TABLE user_ui_catalog_views (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_class TEXT NOT NULL,
    facet TEXT NOT NULL,
    view_mode TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, device_class, facet)
);

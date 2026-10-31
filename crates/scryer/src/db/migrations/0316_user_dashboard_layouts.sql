-- Dashboard layouts are remembered per user and responsive breakpoint. A
-- layout row with no item rows is a saved layout with every panel removed;
-- no layout row means the web shows its default layout.
CREATE TABLE user_ui_dashboard_layouts (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    breakpoint TEXT NOT NULL,
    column_count INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (user_id, breakpoint)
);

CREATE TABLE user_ui_dashboard_layout_items (
    user_id TEXT NOT NULL,
    breakpoint TEXT NOT NULL,
    panel_id TEXT NOT NULL,
    x INTEGER NOT NULL,
    y INTEGER NOT NULL,
    w INTEGER NOT NULL,
    h INTEGER NOT NULL,
    PRIMARY KEY (user_id, breakpoint, panel_id),
    FOREIGN KEY (user_id, breakpoint)
        REFERENCES user_ui_dashboard_layouts(user_id, breakpoint) ON DELETE CASCADE
);

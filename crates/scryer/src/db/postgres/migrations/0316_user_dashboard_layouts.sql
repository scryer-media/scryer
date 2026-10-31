-- Dashboard layouts are remembered per user and responsive breakpoint. A
-- layout row with no item rows is a saved layout with every panel removed;
-- no layout row means the web shows its default layout.
CREATE TABLE user_ui_dashboard_layouts (
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    breakpoint text NOT NULL,
    column_count integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    PRIMARY KEY (user_id, breakpoint)
);

CREATE TABLE user_ui_dashboard_layout_items (
    user_id text NOT NULL,
    breakpoint text NOT NULL,
    panel_id text NOT NULL,
    x integer NOT NULL,
    y integer NOT NULL,
    w integer NOT NULL,
    h integer NOT NULL,
    PRIMARY KEY (user_id, breakpoint, panel_id),
    FOREIGN KEY (user_id, breakpoint)
        REFERENCES user_ui_dashboard_layouts(user_id, breakpoint) ON DELETE CASCADE
);

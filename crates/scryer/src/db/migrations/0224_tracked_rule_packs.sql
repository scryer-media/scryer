CREATE TABLE rule_pack_installations (
    pack_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    digest TEXT NOT NULL,
    auto_update INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    last_updated TEXT NOT NULL,
    last_error TEXT
);

CREATE TABLE rule_pack_members (
    pack_id TEXT NOT NULL REFERENCES rule_pack_installations(pack_id) ON DELETE CASCADE,
    template_id TEXT NOT NULL,
    rule_set_id TEXT NOT NULL REFERENCES rule_sets(id) ON DELETE CASCADE,
    removed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (pack_id, template_id),
    UNIQUE (rule_set_id)
);

CREATE INDEX idx_rule_pack_members_pack_id ON rule_pack_members(pack_id);

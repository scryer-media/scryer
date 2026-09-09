CREATE TABLE rule_pack_installations (
    pack_id text PRIMARY KEY,
    name text NOT NULL,
    version text NOT NULL,
    digest text NOT NULL,
    auto_update boolean NOT NULL,
    revision bigint NOT NULL,
    last_updated timestamptz NOT NULL,
    last_error text
);

CREATE TABLE rule_pack_members (
    pack_id text NOT NULL REFERENCES rule_pack_installations(pack_id) ON DELETE CASCADE,
    template_id text NOT NULL,
    rule_set_id text NOT NULL REFERENCES rule_sets(id) ON DELETE CASCADE,
    removed boolean NOT NULL DEFAULT FALSE,
    PRIMARY KEY (pack_id, template_id),
    UNIQUE (rule_set_id)
);

CREATE INDEX idx_rule_pack_members_pack_id ON rule_pack_members(pack_id);

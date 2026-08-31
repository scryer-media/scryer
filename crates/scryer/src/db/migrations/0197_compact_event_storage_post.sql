DROP TABLE domain_events_legacy_0197;
DROP TABLE release_decisions_legacy_0197;

CREATE INDEX idx_domain_events_occurred_at ON domain_events (occurred_at DESC);
CREATE INDEX idx_domain_events_event_type_sequence ON domain_events (event_type, sequence DESC);
CREATE INDEX idx_domain_events_title_sequence ON domain_events (title_id, sequence DESC);
CREATE INDEX idx_domain_events_facet_sequence ON domain_events (facet, sequence DESC);
CREATE INDEX idx_release_decisions_wanted ON release_decisions (wanted_item_id, created_at DESC);
CREATE INDEX idx_release_decisions_created_at ON release_decisions (created_at DESC);

CREATE TABLE title_images (
  id TEXT PRIMARY KEY,
  title_id TEXT NOT NULL,
  provider TEXT NOT NULL,
  provider_image_id TEXT,
  kind TEXT NOT NULL,
  source_url TEXT NOT NULL,
  source_etag TEXT,
  source_last_modified TEXT,
  source_format TEXT NOT NULL,
  source_width INTEGER,
  source_height INTEGER,
  storage_mode TEXT NOT NULL,
  master_path TEXT,
  master_format TEXT NOT NULL,
  master_sha256 TEXT NOT NULL,
  master_width INTEGER NOT NULL,
  master_height INTEGER NOT NULL,
  bytes BLOB NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE,
  UNIQUE (title_id, kind)
);

CREATE INDEX idx_title_images_title_kind ON title_images(title_id, kind);

CREATE TABLE title_image_variants (
  id TEXT PRIMARY KEY,
  title_image_id TEXT NOT NULL,
  variant_key TEXT NOT NULL,
  path TEXT,
  format TEXT NOT NULL,
  width INTEGER NOT NULL,
  height INTEGER NOT NULL,
  bytes BLOB NOT NULL,
  sha256 TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (title_image_id) REFERENCES title_images(id) ON DELETE CASCADE,
  UNIQUE (title_image_id, variant_key)
);

CREATE INDEX idx_title_image_variants_image_variant
  ON title_image_variants(title_image_id, variant_key);

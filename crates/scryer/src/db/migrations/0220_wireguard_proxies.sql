-- WireGuard joins the tunnel family.
--
-- Everything a WireGuard tunnel needs that an SSH tunnel does not. The two
-- share the columns that mean the same thing in both worlds — `base_url` (the
-- peer's UDP endpoint, as `wireguard://host:port`), `private_key_encrypted`
-- (base64 here rather than PEM), `request_timeout_seconds` — and the columns
-- that mean nothing to WireGuard (`username_encrypted`, `password_encrypted`,
-- `private_key_passphrase_encrypted`, `host_key_fingerprint`,
-- `host_key_pinned_at`, `remote_dns`) simply stay NULL. WireGuard has no user
-- to be, no passphrase format, and no trust-on-first-use step, because the
-- peer's public key *is* its identity and it is configured up front.
--
-- Which of the seven are secret is not uniform, and the split is deliberate.
--
-- * `peer_public_key` and `tunnel_public_key` are **public**, stored in the
--   clear. The first is what the operator pasted out of their server's
--   `[Peer]` section. The second is ours, derived from the private key and
--   maintained by the workflow on every key write, so the operator can compare
--   it against what they put in the *server's* `[Peer]` section. Masking either
--   would hide the one value that makes a key mistake diagnosable.
-- * `preshared_key_encrypted` is a symmetric secret and follows the same
--   at-rest convention as every other `*_encrypted` column.
-- * The remaining four are ordinary configuration.
--
-- `tunnel_addresses` and `tunnel_dns_servers` are comma-separated text rather
-- than a child table. They are short, ordered, always read as a whole, and they
-- are literally what the operator pasted out of an `[Interface]` block.
-- `tunnel_mtu` and `tunnel_keepalive_seconds` admit NULL to mean "the engine's
-- default". A stored 0 keepalive means the operator switched it off, which is
-- a different statement from having no opinion.
ALTER TABLE proxy_configs ADD COLUMN peer_public_key TEXT;

ALTER TABLE proxy_configs ADD COLUMN preshared_key_encrypted TEXT;

ALTER TABLE proxy_configs ADD COLUMN tunnel_public_key TEXT;

ALTER TABLE proxy_configs ADD COLUMN tunnel_addresses TEXT;

ALTER TABLE proxy_configs ADD COLUMN tunnel_dns_servers TEXT;

ALTER TABLE proxy_configs ADD COLUMN tunnel_mtu INTEGER;

ALTER TABLE proxy_configs ADD COLUMN tunnel_keepalive_seconds INTEGER;

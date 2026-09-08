-- Preserve assignments so legacy SSH configurations fail closed until repaired.
UPDATE proxy_configs
SET last_health_status = 'unhealthy',
    last_error_message = 'SSH tunnels require an Ed25519 private key; password authentication is no longer supported',
    last_error_at = now()
WHERE provider_type = 'ssh_tunnel'
  AND (private_key_encrypted IS NULL OR private_key_encrypted = '');

UPDATE proxy_configs
SET password_encrypted = NULL,
    updated_at = now()
WHERE provider_type = 'ssh_tunnel';

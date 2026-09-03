# scryer-v0.19.5

AI generated release notes

Scryer 0.19.5 is a security and reliability release focused on safer account protection, steadier OAuth and plugin flows, and better control over discovery data growth.

## Highlights

- Sensitive account security changes now require fresh verification, covering actions such as passkey and TOTP management.
- Form login can no longer be enabled until an administrator has set a usable password, reducing the chance of exposing password login on an unsecured instance.
- OAuth authorization is stricter and more reliable, with better client and redirect validation, improved session compatibility, and clearer reauthentication behavior when a session is no longer fresh.
- Plugin uploads and command-plugin descriptor loading are more reliable, fixing failures around uploaded plugin payload handling.
- Discovery retention is now enforced to keep metadata storage from growing without bound.
- RSS polling now returns to its expected cadence after failures, and Unix download timestamps are handled correctly.
- Includes backend reliability work such as a slow-query fix.

## Security and hardening

- Closes multiple authentication race conditions and hardens login, passkey, and TOTP factor-change boundaries.
- Tightens rate limiting around authentication and OAuth flows, including safer handling when Scryer is deployed behind reverse proxies.
- Embeds verified Sigstore trust material to strengthen builtin trust verification.

## Upgrade notes

- New configuration: `SCRYER_RATE_LIMIT_TRUSTED_PROXY_IPS`. Set this only to proxy IPs or CIDRs you control if you rely on `X-Forwarded-For` for client-aware rate limiting.
- This release includes database migrations for bounded discovery storage, account security factor state tracking, and OAuth authorization-code session epoch compatibility.
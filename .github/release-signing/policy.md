# Scryer release signing policy

Official Scryer release tags use the SSH signing identity in `allowed_signers`.
Its OpenSSH fingerprint is `SHA256:ceWh+l4B+GyQRDjMoir5qgYpUTeS5omk6D/STEqsL5g`.

The `verify-release-trust` job configures Git to use this file and runs
`git verify-tag` for every `scryer-v*` tag. It then derives the matching
`release-X.Y.Z` branch, rejects tags not reachable from that branch, and requires
that the release branch include the current `origin/main`. Consumers can use the
same file as their Git SSH allowed-signers file when independently verifying a
release tag.

The matching GitHub tag ruleset permits a release tag's initial creation, but
blocks updates and deletion. Artifact and OCI signatures are keyless Sigstore
signatures issued from GitHub Actions OIDC; no artifact-signing private key is
stored in this repository or its release environment.

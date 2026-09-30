# Scryer 0.21.13 release notes

These notes cover what's changed since **0.21.12**.

## Highlights

- **Grab logging is quiet again at the default level.** The per-stage lines that 0.21.12 added to trace a stalled grab submission are now logged at debug rather than info, so ordinary operation no longer writes a dozen lines for every grab. They remain available by raising the log filter for the `scryer_application::acquisition::submission`, `scryer_application::catalog::workflow` and `scryer_infrastructure_acquisition::downloads::clients` modules.

## Included fixes

- **Release validation:** a test fixture for outbound HTTP cooldown handling closed its connections without saying so, which could make a later request in the same test land on a stale pooled connection and fail intermittently. The fixture now marks each response `Connection: close`.

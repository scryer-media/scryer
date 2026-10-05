# Scryer 0.21.13 release notes

These notes cover what's changed since **0.21.12**.

## Highlights

- **Security update for the plugin sandbox.** Wasmtime and WASI are updated to 48.0.4, fixing vulnerabilities that could let a malicious plugin escape the WebAssembly sandbox or exhaust host memory. Operators running plugins should upgrade promptly.
- **Frontend tooling security:** the component generator CLI and its vulnerable dependencies are removed. Its Tailwind utilities are preserved locally, keeping the web app's styling unchanged.

## Included fixes

- **Dashboard:** free space below 1 TB is shown in gigabytes instead of a fraction of a terabyte.

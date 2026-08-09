# Contributing to LadoFlow

LadoFlow is at the architecture and foundation stage. Small, measurable changes are more useful than broad rewrites.

## Before opening a change

1. Check existing issues and the roadmap.
2. State the host OS, display device, connection type, and test hardware when reporting platform behavior.
3. Keep protocol changes versioned and include round-trip tests.
4. Do not add analytics, cloud dependencies, or account requirements to the core path.
5. Never commit signing keys, certificates, device identifiers, or captured user content.

## Local checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Platform-specific projects will add their own checks when they become buildable.

## Commit style

Use focused, imperative commit subjects, for example:

- `Define versioned frame header`
- `Add Android decoder capability probe`
- `Measure loopback frame latency`

Include evidence in the pull request description: commands, hardware, output, and known limits.


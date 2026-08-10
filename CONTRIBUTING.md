# Contributing to LadoFlow

LadoFlow is at the architecture and foundation stage. Small, measurable changes are more useful than broad rewrites.

## Before opening a change

1. Choose a full or platform-only checkout using the [source checkout guide](./docs/source-checkout.md), then check existing issues and the roadmap.
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

Run the checks for every platform touched by a change.

Android changes use the Gradle wrapper from `apps/android`:

```bash
cd apps/android
./gradlew --no-daemon testDebugUnitTest lintDebug lintRelease assembleDebug assembleRelease assembleDebugAndroidTest
```

The release APK produced by repository CI is intentionally unsigned. Never add
release signing keys or machine-specific `local.properties` files to Git.

## Commit style

Use focused, imperative commit subjects, for example:

- `Define versioned frame header`
- `Add Android decoder capability probe`
- `Measure loopback frame latency`

Include evidence in the pull request description: commands, hardware, output, and known limits.

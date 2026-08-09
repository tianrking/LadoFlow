# ladoflow-core

Dependency-light shared policy for LadoFlow endpoints. The crate provides:

- protocol-version and capability negotiation over `ladoflow-protocol` hellos;
- an explicit session/reconnect state machine with stream continuity rules;
- bounded rolling latency aggregation; and
- a deterministic latency-based quality recommendation.

It intentionally performs no I/O, owns no transport, and starts no timers.
Callers drive state transitions and apply the returned reconnect delays and
quality recommendations in their platform-specific runtime.

Run the isolated checks from this directory:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

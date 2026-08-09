# Shared Rust crates

Crate boundaries:

- `ladoflow-protocol` — wire types and bounded binary framing (foundation created);
- `ladoflow-core` — session state, negotiation, telemetry, and quality policy;
- `ladoflow-transport` — transport traits and test/loopback implementations;
- `ladoflow-media` — codec-neutral media metadata and frame scheduling.

Platform drivers and mobile rendering do not belong in these crates.

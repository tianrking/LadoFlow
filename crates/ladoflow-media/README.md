# ladoflow-media

`ladoflow-media` contains the platform- and codec-neutral pieces of LadoFlow's
media path:

- video dimensions, rational frame rates, timestamps, and frame identity;
- deterministic synthetic payloads for repeatable loopback diagnostics;
- a one-frame scheduler with exact rational pacing and bounded payload size;
- counters and latency summaries for stale, superseded, oversized, and late
  frames.

The scheduler uses caller-supplied `Duration` values as a monotonic clock. This
keeps tests deterministic and lets desktop integrations use their native event
loop or display clock without this crate owning threads or sleeping.

The crate intentionally has no dependency on `ladoflow-protocol`. A transport
adapter can map stable media metadata into whichever protocol generation is
currently negotiated.

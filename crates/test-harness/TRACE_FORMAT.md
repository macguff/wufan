# Deterministic trace format

`replay` emits a binary, little-endian format. Version 5 is a development artifact format; it is not a stable IPC wire protocol.

```text
"wufan-trace-v5"
u64 initial_state_length
byte[initial_state_length] canonical RuntimeState ("wufan-runtime-state-v5")
repeat for each input event:
    encoded Event (tagged; event-specific fixed/optional fields)
    u64 transition_length
    byte[transition_length] canonical TransitionResult
```

Event tags are append-only within a trace major version:

| Tag | Event |
| --- | --- |
| 0 | HostKey (`KeyPhase`: Test=0, Actual=1) |
| 1–3 | FocusGained, FocusLost, BrokerDisconnected |
| 4 | EffectResult (`class`: Termination=0, Start=1, Update=2, HostCommit=3; `outcome`: Succeeded=0, Rejected=1, Indeterminate=2) |
| 5–9 | CommitIntent, BrokerReady, RequestCompleted, PumpMutatingQueue, EngineUpdate |
| 10–13 | DeadlineExpired, CandidateViewUpdated, UIActionIntent, PhysicalKey |
| 14–15 | HostClosing, InputMethodDeactivated |
| 16 | HostKeyUp (`KeyPhase`: Test=0, Actual=1) |
| 17–18 | ContextPushed, ContextPopped |
| 19 | DecisionTtlExpired (`token`, `now_ms`) |

All integer fields are unsigned little-endian unless their Rust field type is signed. Optional values carry a one-byte presence tag. Effect-scope tags are Composition=0 and Commit=1. Commit scope includes the client incarnation ID as well as session, epoch, and commit ID.

Trace payloads contain synthetic token IDs, key observations, and model IDs only; they must never contain typed text, committed strings, or user dictionary content. For deterministic regression, store the initial state and event sequence; for schema changes, bump the version prefix rather than reinterpreting prior bytes.

`decode_trace_events(initial_state, bytes)` validates the v5 header and canonical initial-state bytes, decodes each input event, and skips the recorded transition payload. Replaying the returned events with `replay` must reproduce the entire canonical trace byte-for-byte. `run_with_fake_adapter` is the deterministic test scheduler: it reduces each supplied input, executes its effects through the fake adapter, and reduces generated response events before the next supplied input.

The seeded runtime explorer minimizes reducer panics and writes the canonical binary trace before failing. Set `WUFAN_TRACE_ARTIFACT_DIR` to choose the output directory; otherwise it writes under `crates/test-harness/regression-traces/`. CI uploads that directory even when the core test step fails. Artifacts contain only synthetic token IDs and model events, never host text.

# Slice 0 scenario evidence

This is model/fake-adapter evidence only. It does not claim Windows TSF, named-pipe, signer, callback-latency, or native-host validation. Each row maps a frozen scenario in `1.md` to executable tests.

The commit reducer/effect scope and fake host journal bind writes by `(client_instance_id, session_id, commit_id)`. `cross_model_i2_i4_i5_i6_oracle_checks_commit_deployment_and_memory_facts` runs success, rejection, both indeterminate commit outcomes, D6 recovery, and EventDot duplicate import through one invariant-focused oracle. I3 stale-identity gating is tested separately.

| ID | Executable evidence | Result / remaining scope |
| --- | --- | --- |
| M01 | `m01_thirty_keys_per_second_remain_ordered_and_unique` | Passes synthetic 30-key/second ordered trace; no real-host latency claim. |
| M02 | `mutating_deadline_fails_to_passthrough_and_ignores_late_reply`; `required_deadline_values_preserve_m02_m05_m19_failure_invariants` | Passes model timeout and late-reply drop; stall timing is not measured. |
| M03 | `m03_old_broker_generation_response_is_dropped_after_restart` | Passes generation restart model. |
| M04 | `mutating_requests_are_queued_in_order_with_a_single_request_in_flight`; `queue_overflow_terminates_active_composition_and_fails_open` | 64 queued mutations reach the bounded limit; the next test callback returns Pass, clears unsent work, invalidates the session, and terminates any active composition. |
| M05 | `m05_m06_crash_after_host_write_does_not_reapply_old_incarnation_commit` | Passes Apply-once under pending duplicate and simulated lost terminal response. |
| M06 | `m05_m06_crash_after_host_write_does_not_reapply_old_incarnation_commit`; `stale_commit_identity_is_rejected_before_host_effect` | Passes new-incarnation stale intent rejection. |
| M07 | `fake_adapter_observes_at_most_one_host_mutation`; `successful_commit_is_terminal_and_duplicate_only_resends_ack` | Passes duplicate intent/result handling and host journal at-most-once check. |
| M08 | `m08_out_of_order_completion_cannot_advance_the_mutation_queue` | Passes out-of-order request completion gate. |
| M09 | `stale_commit_identity_is_rejected_before_host_effect` | Passes forged generation rejection. |
| M10 | `focus_loss_cancels_composition_as_a_single_obligation_effect`; `context_push_terminates_old_context_without_orphaning_start_effects`; `mismatched_focus_loss_is_diagnostic_and_new_focus_preserves_pending_cancel` | Passes focus/context invalidation, composition cancellation, late-start cleanup, and new-focus preservation in the model. |
| M11–M12 | `host_close_and_deactivation_never_wait_and_cancel_active_composition` | Passes non-blocking-effect model for deactivation/host close. |
| M13 | `unmatched_or_expired_actual_callback_fails_open`; `decision_ttl_event_expires_only_the_matching_elapsed_entry` | Passes expired/unmatched decision fail-open and explicit timer behavior, including early/duplicate timer no-op. |
| M14 | `ambiguous_physical_key_match_fails_open` | Passes ambiguous key correlation fail-open. |
| M15 | `full_ledger_passes_without_eating_the_key` | Passes decision-ledger capacity behavior. |
| M16–M17 | `candidate_intents_share_the_mutation_sequence_and_reject_stale_views` | Passes shared ordering and stale view revision rejection. |
| M18 | `stale_focus_or_geometry_revision_never_repositions_current_candidate` | Passes geometry model only; no HWND/DPI integration. |
| M19 | `required_deadline_values_preserve_m02_m05_m19_failure_invariants`; `d5_without_d6_recovers_committed_deployment` | Passes pre-D6 recovery-to-A model. |
| M20 | `d6_persisted_deployment_survives_restart` | Passes post-D6 recovery-to-B model. |
| M21 | `activation_waits_until_every_old_session_is_cancelled` | Passes modeled drain-before-activate gate. |
| M22 | `failed_d6_rolls_back_or_stays_in_passthrough` | Passes rollback failure safe-mode model. |
| M23 | `a_b_c_a_and_repeated_import_apply_each_dot_once` | Passes EventDot idempotence model. |
| M24 | `same_sid_untrusted_client_is_not_a_broker` | Passes signer-policy model; no real pipe ACL or OS token check. |
| M25 | `developer_build_requires_explicit_dev_trust_mode` | Passes explicit dev-trust policy model. |
| M26 | `modifier_release_bookkeeping_is_independent_of_mutating_queue_pressure`; `physical_modifier_map_tracks_each_side_and_ignores_non_modifiers`; `key_up_callbacks_match_only_key_up_ledger_entries_and_preserve_direction` | Passes left/right modifier tracking, key-up ledger/effect-direction models, and queue-pressure cleanup; real TSF delivery and host key-up behavior still need Windows verification. |

## Slice 0 exit gaps

- Runtime random exploration uses 16 × 1,000 fixed events plus an optional fresh CI nightly seed; it asserts coverage of every Event variant, result class, and result outcome. A 32-case `proptest` generates 1,000–1,256-event Runtime traces, checks S1–S3 and the forbidden failed-state combinations after every prefix, and verifies trace decode/replay; the same inputs also run through the fake scheduler and check Applied/journal agreement and unique ApplyHostCommit identities. Deployment and Memory models run seeded 32 × 1,000 fault/merge interleavings; deployment checks phase-local D6 invariants after each action. Failures shrink through proptest or the runtime trace shrinker.
- Matching Rust 1.91.1 `llvm-tools-preview` coverage runs locally. Combining the harness and RuntimeCore test binaries reports 90.87% region and 93.95% line coverage across core/models (RuntimeCore: 90.76% region, 93.67% line). LLVM's stable report here exposes no branch counters, so branch coverage is not claimed. CI now combines both test binaries and filters dependency sources; remote artifact publication is still unverified.
- I2/I4/I5/I6 have a combined cross-model oracle. Proptest varies commit outcomes, verifies duplicate intent/result non-reapplication, Applied ACK/journal consistency, and mutates all six commit identity fields to verify I3 rejection before host mutation. A deterministic regression now covers Broker disconnect during a pending TSF composition start and late success/cancel; minimized reducer-panic traces are persisted and CI uploads them on failure. Full reducer transition coverage and verification of the remote CI artifact remain open.
- S1–S3 plus focus/session/composition/mode consistency are checked by the reducer assertions and every generated trace prefix. Full reducer transition/branch coverage is not yet demonstrated. No failure trace was produced by the passing local run, so the new failure-artifact upload path still needs remote CI verification.
- I1, actual COM edit-session behavior, OS-level authentication, sanitizer/process-kill behavior, and all Windows-native evidence remain open integration gates.

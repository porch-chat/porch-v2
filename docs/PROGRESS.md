# Implementation progress

## 2026-11-06

Phase 0 foundations, Phase 1 identity and Phase 2 gateway are complete with green
GitHub-hosted CI. Phase 3 is complete in the same worker, no deployments.

CI speedup commit 2e5f1ae retains every ignored DB test serially, CLI smoke and
single hosted job. Baseline run 42173780003 took 157s workflow / 154s job without
Rust caching. Optimized cold run 42175402463 attempt 1 took 274s workflow (cache
population and nextest compile mode made cold slower). Warm attempt 2 succeeded
in 81s workflow / 76s job. No self-hosted runners or global git config changes.

Phase 3 protocol/permissions commit f906231 and persistence/authorization commit
0bde930 are locally green. CLI lifecycle smoke and integration coverage added.
Community defaults, category-sync semantics, bounded aggregate row locks,
idempotent official seeding, invitation races and payload-free replay-safe
invalidations are documented in ADR-0004 and communities.md. Phase 3 workspace/DB/CLI checks passed, 21 default tests and 5 ignored DB tests.
Hosted CI 42176668393 at 6708ce8 succeeded in 111s workflow / 92s job.
Exact Phase 3 commits are f906231 (contracts/engine), 0bde930 (persistence/HTTP),
54b8ddb (CLI/tests/docs). Green run https://github.com/porch-chat/porch-v2/actions/runs/42176668393
also includes branding 6708ce8. The prior phase run was cancelled during cache
upload by branding after its test steps passed, so only the replacement is called
green. No graphical, deployed or live-user acceptance has occurred.
Phase 4 messaging has started, design in ADR-0005.

Dylan explicitly approved sequential continuation through later phases, with
local tests and green hosted CI before each next phase. No extra workers, no PC
access, no dedicated-server deployments, DNS, Caddy or infrastructure changes.
Deployment-dependent live validation must be marked blocked, never implied.

For Phase 6, design inspiration only is the familiar Discord-style server rail,
channel sidebar, message pane and member list (Fluxer/Sharkord are UX references,
not code sources). Porch should feel warm and close-knit, with its own character.
Read assets/brand/BRAND.md and use the concurrent branding assets when present;
never replace branding task files. Load frontend-design before UI coding, write
a design ADR, build Linux/Windows artifacts or honestly document build blockers.
Headless screenshots where feasible; graphical acceptance belongs to Dylan alone.
Before every push, safely commit local work then pull --rebase to preserve branding.

Phase 4 implementation commits 7fa6b31 (shared messaging/sync contracts) and
4aad12a (Postgres persistence, authorized HTTP, DM lifecycle, isolated integration).
CLI proof and docs are commit fc76564. Local fmt/clippy/build/workspace tests passed, 25 default
tests and 6 ignored integration/load tests, including 256 sanitizer property cases
and 5 permission properties with 512 cases each. Actual CLI identity+community+
messaging HTTP smoke passed. A 20k synthetic-row FTS warm serial benchmark measured
12.873ms p50 / 14.664ms p95 over 50 authorized full-HTTP calls in the debug build,
not production/concurrent capacity. Local build cache exhausted disk; only this
repo's generated target was cleaned, then all tests rebuilt without debuginfo.

Phase 4 hosted CI passed at fc76564, 107s workflow / 104s job with an existing
cache and new dependencies. Run https://github.com/porch-chat/porch-v2/actions/runs/42177860647
Phase 5 starts after this green gate.
See docs/messaging.md and ADR-0005 for bounds, tombstone/sanitizer semantics,
revision paging and known boundaries. No outbox/cross-node guarantees, no E2EE,
no group consent/blocking, no real-user or deployment acceptance yet. Phase 2
presence/typing remains platform-level (no channel/content identifiers); scoped
conversation typing is not a Phase 4 claim. No UI, deployment or infrastructure
changes were made.

Phase 5 shared client engine is implemented in 4f3b14b. Account-bound SQLite,
durable optimistic outbox, authorized revision sync, offline profile restore,
reconnect/resume with ephemeral sequence gaps, heartbeat isolation and revocation
purges have local unit/property/fault-injection/real-Postgres coverage. CLI core
commands and actual multi-process smoke are being finalized. No UI started;
Phase 6 must wait for green hosted CI. ADR-0006 and client-engine.md document
last-known offline authorization, plain-text cache, no eviction, initial full-delta
bootstrapping, and deliberate 401 draft-loss boundary.

Phase 5 completed with 4f3b14b and bc8f6c4. Local fmt/clippy/build passed,
33 default tests and all 7 ignored real-Postgres/integration/load tests passed,
including actual multi-process CLI cache/queue/flush/watch/401 smoke. Hosted CI
https://github.com/porch-chat/porch-v2/actions/runs/42179324870 passed, 144s workflow /
141s job, existing restored cache with new dependencies (not controlled warm vs cold).
Forced sign-out purges plaintext drafts by design and records a durable nonce-only
removal notice, never a false promise that remote delivery did not happen. No
refresh-token API (30-day opaque sessions), E2EE, OS credential store or UI claim.

Phase 6 begins after that green gate. Read frontend-design and brand guidance.
Native GPUI desktop, familiar community/channel/message/member layout, original
Evening Threshold styling. Windows compile/artifact evidence is distinct from
runtime/IME/DPI acceptance, which remains Dylan's. No PC access or deployment.

Phase 6 native GPUI client implementation is locally green. Original brand assets
are embedded, real porch-core powers login/register, communities/channels, DMs,
history, durable send/retry/discard, reply/edit/reactions and display settings.
Per-conversation in-memory drafts and explicit send/edit acknowledgements avoid
clearing unsaved text on failures. Secret values never enter UI snapshots or
preferences. Details and known v1 limits are in desktop.md and ADR-0007.
Local fmt/clippy/build, 36 default tests, all 8 ignored Postgres/integration/load
tests and actual multiprocess CLI smoke passed. Session bridge has actual HTTP/WS
and Postgres tests, no fake chat data. Linux native startup with Xvfb/Mesa and a
real loopback backend exited cleanly. Xvfb alone initially captured black frames;
adding Openbox fixed rendering and an inspected screenshot shows the native
layout. A real mouse click/type/Enter through the composer produced a real
backend-acknowledged cached message. This is headless software-rendered evidence,
not graphical hardware/IME/DPI or Windows runtime acceptance. System fallback fonts
are a native prerequisite in addition to the embedded application font.
Hosted Linux + Windows build/artifact CI is pending. Windows compilation,
real Windows/Linux graphical/IME/DPI acceptance and runtime hardware testing are
separate gates. No PC, deployments, DNS, Caddy or infrastructure were touched.

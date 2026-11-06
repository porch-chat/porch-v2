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

Phase 6 follow-up adds a reproducible headless native smoke to hosted Linux CI.
Xvfb + Openbox + Mesa drives the actual username/password form and composer,
then verifies delivered text through the real CLI. Fixture credentials use stdin,
never command-line arguments. A security regression test rejects origin URLs
containing credentials/query/fragment before saving local preferences, and drops
invalid legacy preferences on load. Local checks now pass 37 default tests plus
8 ignored Postgres/integration/load tests. Native smoke passed against a fresh
empty local database. Initial cross-platform hosted build is still running;
do not start Phase 7 or claim Windows acceptance before it passes.

Phase 6 final hosted run 42184052969 passed Linux and Windows at dbdcac9.
Native Linux headless interactive login/composer smoke passed; actual hardware,
Windows runtime, DPI and IME acceptance remain separate Dylan checks.

Phase 7 in progress, not complete. Storage contracts and atomic private local
storage landed first. Backend now has scoped resumable 1 GiB uploads with 4 MiB
chunks, digest finalization, ownership/live authorization, quota/TTL handling,
transactional attachment binding and authenticated streamed ranged downloads.
CLI upload/create/resume/status/abort/download drives the actual endpoints.
Local fmt/clippy/workspace tests/build pass, including all 10 explicitly ignored
integration tests and the actual 1 GiB streaming storage test. Fresh-database
CLI smoke proves interrupted upload plus new-process resume, attachment-only
send, byte-exact download and overwrite refusal. This is backend/CLI evidence,
not the full phase. Shared-core durable transfers, image/video previews,
guarded server-side embeds and native attachment UI remain pending. See files.md
and ADR-0008 for retention bounds and crash behavior. No deployment.

Phase 7 follow-ups (still incomplete): 71fba23 implements authenticated bounded
raster thumbnails; 7586e6f fixes a hosted Windows unused-mut failure in storage
creation, found in eeeb960 run 42186179903 (Linux passed, Windows did not).
Guarded opt-in link metadata now has policy/response limits and actual
Postgres/HTTP authorization tests. CLI message-embed CONVERSATION MESSAGE uses
PORCH_LINK, with no arbitrary unauthenticated URL fetching. Remote network success,
native attachment UI and video previews remain separate unfinished work.

Phase 7 native files (in progress): core media commit 6e1ad71 adds verified
streaming save and bounded authenticated PNG loading. Desktop now has native
choose/save dialogs, separate stage/upload/resume/send/cancel actions, committed
byte progress, received attachment cards and opt-in link metadata/image preview.
Disposable Postgres + actual HTTP desktop worker test passes stage(no upload),
resume/finalize/send, thumbnail, exact save, cancel and logout purge. This is not
interactive platform-dialog or Windows runtime acceptance. Video preview remains
unfinished; Phase 7 is not complete.


Phase 7 optional video posters implemented locally. Generated fixture passes the
real FFmpeg/HTTP/Postgres path; default-disabled, missing-auth/wrong-scope/deleted
denials and malformed/oversized/concurrency guards pass. Linux opt-in only,
resource-bounded pipe decoding with explicit caveats: not an OS sandbox, needs
operator isolation before untrusted use, no actual deployment. Static first-frame
previews, not playback. Final headless media proof and hosted CI still pending;
Phase 7 is not yet complete.

Phase 7 headless native media proof now passes locally. A generated MP4 fixture
is uploaded and attached through the real CLI/backend, the native GPUI card is
rendered, then a real xdotool click requests its authenticated PNG poster. Two
interior pixels verify the expected amber output, and the inspected screenshot
is docs/evidence/phase7-linux-media.png. The repeatable CI harness is
scripts/desktop-media-smoke.py. This proves static poster rendering, not playback
or platform picker/hardware acceptance. All 12 explicit ignored tests passed
across the final local runs; a full-storage test initially hit local ENOSPC,
then passed after clearing only generated build artifacts. Final hosted CI remains
the last automated Phase 7 gate. No deployment or infrastructure work occurred.

Hosted a7854c7 passed Windows but failed the Linux video lifecycle preview
(HTTP 400 instead of PNG). Linux FFmpeg may load threaded numerical libraries;
its cleared child environment now explicitly caps OpenBLAS and MKL threads as
well as OpenMP. Hosted rerun must prove this, not assume a diagnosis.

Diagnostic hosted run 42191764371 confirms FFmpeg 8.0.1 on the hosted image
fails pthread_create while starting the output consumer, not an authorization
or media-signature failure. Exact allocator/stack contribution is not proven.
Local FFmpeg 6.1.1 reproduces thread/resource failure with a 96 MiB inherited
thread stack under the unchanged 512 MiB address-space cap. Decoder children
now explicitly cap stack at 8 MiB and glibc arenas at two, reducing resource
reservations rather than relaxing any decode limits. The bounded CI diagnostic
reports inherited stack limits; hosted validation of this fix remains pending.


Phase 7 final automated gate passed at 620bbca. Hosted run
https://github.com/porch-chat/porch-v2/actions/runs/42193393398
succeeded for both Linux (468 seconds) and Windows (219 seconds); workflow
duration was 471 seconds. Linux passed all 49 default and all 12 explicitly
ignored tests, real CLI smoke, native keyboard smoke and authenticated native
poster pixel proof. Local fmt/clippy/workspace tests/build and the same 12
ignored tests also passed after the decoder reservation fix. No decode safety
limit was raised. The exact allocator/stack cause remains unproven, despite
confirmed prior hosted pthread_create failure and the now-green bounded fixture.

Linux debug build and hosted native screenshots:
https://github.com/porch-chat/porch-v2/actions/runs/42193393398/artifacts/13491626131
Windows debug build:
https://github.com/porch-chat/porch-v2/actions/runs/42193393398/artifacts/13490404786
Private artifacts expire after seven days. Local inspected poster evidence is
docs/evidence/phase7-linux-media.png; scripts/desktop-media-smoke.py repeats
the actual authenticated request, native click and two amber-pixel assertions.

This completes Phase 7's implemented automated scope, not graphical user
acceptance. Static posters are not playback; FFmpeg is default-off opt-in and
not an OS sandbox. Remote embed success, interactive picker/save dialogs,
Windows runtime and real hardware/DPI/IME acceptance remain unclaimed or Dylan
checks. The 1 GiB proof is streaming storage, not full HTTP/UI. No deployment,
PC access or infrastructure changes occurred. Phase 8 deployment is not
authorized. This final record is a docs-only change; hosted CI ignores Markdown
under docs, so 620bbca is the tested code parent rather than a new CI run.

## Phase 8 control-plane checkpoint (not phase completion)

Porch-owned VoiceProvider seam, LiveKit microphone-only short-lived tickets,
VIEW_CHANNEL/CONNECT/SPEAK authorization, session-bound leases, no-store snapshots,
leave/revocation/replayed-ticket SFU inventory cleanup and payload-free gateway
invalidations implemented. CLI voice-join/snapshot/state/leave added; credentials
exported only by explicit voice-join stdout. Disposable Postgres voice integration
passed including authentication, private channel denial, duplicate joins, session
isolation, deafen normalization, RPC failure retry and session revocation.
Native media/UI, hosted CI and isolated deployment remain pending. Deployment
runbook is docs/deploy/phase8.md. No public DNS change authorized or performed.

Phase 8 native implementation checkpoint: official LiveKit native audio adapter
now wired into desktop join/leave, mic mute, deafen, device cycling, focused F9
push-to-talk and participant/speaking display. UI-side synchronous track gate
silences on release/mute even when session worker is busy. New media starts muted.
Native cargo check passed with clang 21 + GLib; full workspace checks and hosted
Linux/Windows acceptance still pending. Isolated backend 11a4cfb deployed and real
control-plane smoke passed; public API hostname conflicts with old Fluxer route,
so no existing service route was changed. DNS voice added, api already correct.

## Desktop design polish (2026-11-06)

Native Evening Threshold conversation layout now uses compact author/time groups, deterministic initial avatars, embedded line icons, hover-only message actions, community selection/unread pills, collapsible channel categories, inline connected voice participants, role-grouped members, and the bottom user/composer panels. No server contracts or backend behavior changed. Missing member names and live text presence remain explicitly unavailable rather than fabricated (ADR 0010).

Proof: `docs/evidence/ui-polish-conversation.png`, `ui-polish-hover.png`, and `ui-polish-media.png`. The first two use four real disposable accounts, eight API-created messages, categories, text/voice channels and a downloaded-file card; the media proof explicitly loads the authenticated generated video poster. `scripts/desktop-polish-proof.py` reproduces the conversation fixture. No active microphone/hardware or Windows runtime acceptance is implied. The existing native keyboard/login/send smoke and authenticated poster smoke passed locally, along with fmt, strict workspace clippy, workspace tests and build. Hosted Linux/Windows acceptance is tracked against the final commit, not claimed from these local checks.

UI review follow-up: studied Sharkord's layout source read-only and reimplemented column/header/footer geometry, compact groups and integrated composer in GPUI. Added NOTICE attribution. Removed decorative taglines and explanatory panel notes; functional selected-channel placeholder. Medium body / semibold names, brighter secondary token (minimum8.58:1 on dark surfaces), compact file card and save icon. Re-captured conversation/hover as native client windows excluding Openbox external frame; authenticated media poster and actual login/keyboard/send smoke pass with updated coordinates.

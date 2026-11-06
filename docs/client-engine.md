# Shared client engine

Phase 5 is porch-core, no UI or deployment. All wire structs come from
porch-protocol. SQLite, HTTP, optimistic sends, current authorized sync and one
reconnecting gateway worker are shared by CLI and future clients.

Set PORCH_URL, PORCH_TOKEN and PORCH_CACHE (an explicit private database path).
Credentials remain in process environment/memory, never command arguments, URLs,
SQLite or printed transport errors. Cache contains readable messages/drafts.
Use an OS-private directory, not an untrusted shared/symlink-writable location.
HTTPS/WSS is required except loopback. Redirects are refused.

    porch-cli core-sync
    porch-cli core-cache
    porch-cli core-messages CONVERSATION_ID
    PORCH_MESSAGE='{"nonce":"ignored_by_core","content":{"format":"markdown","text":"hello"},"reply_to":null}' porch-cli core-queue CONVERSATION_ID
    porch-cli core-flush
    porch-cli core-retry NONCE
    porch-cli core-discard NONCE
    PORCH_WATCH_SECONDS=30 porch-cli core-watch
    porch-cli core-logout

core-queue generates a random nonce, stores before sending and does not require
network after cache initialization. core-flush first reauthenticates/reconciles,
then submits queued sends. A new process keeps the same durable queue. Timeouts,
429 and 5xx keep queued; other 4xx fail explicitly without blocking later sends.
Retry reuses the same immutable body/nonce, never silently rewrites an uncertain
send. To change a rejected draft, discard and queue a new body. A committed send
whose response was lost is acknowledged by later delta or idempotent retry.

core-watch is bounded to 1..3600 seconds for terminal proof, defaults 30, outputs
observed connection states plus final heads/pending. The engine itself runs until
its GatewayTask is dropped/stopped. Exactly one worker can run per Engine; use
separate login sessions for different devices/processes per gateway protocol.

Opening attempts /auth/me. Transport/429/5xx can restore an already initialized
same-origin profile/cache offline. A fresh uninitialized database requires online
authentication. The cache refuses a different account or origin. Every reconcile
verifies the bound account again before sending anything. 401/logout clears
conversations, messages, snapshots and drafts. Revoked/missing conversations
cascade their cached messages and queued sends. This intentionally sacrifices
unsent draft recovery on credential revocation; no refresh tokens exist.

Head reconciliation is complete/permission-filtered. Revision delta pages apply
idempotently; durable cursor advances only at the final page. POST acks cannot
skip earlier messages. Tombstones replace old content, out-of-order revisions
cannot resurrect it. A lower server head resets that conversation on restore.
The gateway accepts increasing sequence gaps because typing is not replayed,
resumes in memory, and refetches on fresh Ready or safe invalidations. Heartbeats
run independently from serialized sync; a 15-second periodic pass covers missed
invalidation. Reconnect uses bounded exponential jitter, authentication failure
stops retries. Only heads/changed deltas are fetched, not entire histories on
every invalidation. Current SQLite bootstrapping does fetch the complete existing
message delta history; retention/initial-tail optimization remains future work.

Offline data is last-known state, not current permission. Local cache is not
E2EE or encrypted at rest. Deletion is logical, not secure storage erasure.
There is no cache eviction/retention, cross-process engine lease, background OS
service, refresh flow, production capacity claim or graphical acceptance yet.

Tests use temporary SQLite, a deterministic fake WebSocket, and an isolated
schema on disposable Postgres with actual loopback HTTP/WebSocket. They exercise
lost commit acknowledgement, interrupted multi-page sync, nonce dedupe, delayed
acks, edit/tombstone ordering, durable restart, offline profile restore, failed
send isolation, revocation purge, wrong-account protection, resume gaps, task
ownership/cancellation and unauthorized retry termination. Run ignored tests
serially with DATABASE_URL, as CI does; scripts/identity-smoke.py proves the
actual CLI core commands alongside identity/community/messaging flows.

A 401 can also be normal 30-day session expiry. Sign-out errors report removed
local-send count, never guaranteed delivery failure. A request may already have
committed. The latest nonempty removal notice persists conversation IDs/nonces,
time and an unknown-delivery flag in binding metadata without draft content;
core-cache exposes it after re-login and clients acknowledge it after display.
Do not blindly retype/resend these with a fresh nonce. Token expiry is not exposed
by the opaque token/current auth API, so no local expiry calculation is possible.

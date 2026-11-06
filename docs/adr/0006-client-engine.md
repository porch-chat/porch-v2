# ADR-0006: Shared client engine and account-bound SQLite
Status: accepted for Phase 5
Date: 2026-11-06

porch-core owns async HTTP, gateway reconnect/resume, reconciliation, persisted
optimistic sends and account state. UI and CLI consume it; neither implements a
second sync engine. SQLite through existing SQLx keeps maintained async drivers.
Use one SQLite connection with foreign keys and WAL; bind the database to exactly
one origin and user ID and reject mismatches. Bearer tokens remain in memory,
never in URLs, cache, Debug, logs or transport error text. HTTPS required outside
loopback, same-origin WSS, redirects disabled and server certificates verified.

An async engine mutex serializes network/cache reconciliation and outbox sends.
Heads is a complete, READ_HISTORY-filtered authoritative list; only successful
complete responses can purge missing conversations. Community snapshots are
permission filtered. Cache upserts use message revision, not arrival ordering.
Delta pages sort by revision, cap at a captured high watermark, apply idempotently
one page at a time, and advance the durable cursor only after the final page.
Edits that move past the high watermark arrive during the next pass. Server
restores (head below cache cursor) reset that conversation before resync. Server
tombstones are retained without compaction in Phase4, so deletion is observable.

Outbox sends are durable before network transmission and use a random nonce.
Server nonce+fingerprint idempotency handles timeout after commit. A delta or POST
ack removes only that user's matching nonce. Acks never advance sync cursors,
so preceding messages cannot be skipped. Transport/429/5xx leave queued; permanent
4xx mark failed for explicit retry or removal. Failed messages do not block later
sends. Reconcile fresh permissions before flushing, purge revoked conversations
and their outboxes. Explicit logout or 401 purges cache/outbox and stops the engine;
this deliberately trades unsent draft recovery for fail-closed revoked sessions.
No refresh flow exists yet. Document this product boundary, do not imply DRM.

One gateway task per engine sends heartbeats, verifies ordering, resumes from an
in-memory cursor during reconnect, resets on fresh Ready and coalesces invalidation
wakeups through serialized sync. Persisting cursor is unnecessary: process startup
reconciles anyway. Periodic reconciliation recovers missed publication/outbox gaps.
Cancellation owns and stops the task; callers must not spawn duplicate gateway
workers. Exponential capped jittered backoff avoids reconnect storms. 401/Forbidden
must never retry an invalid credential indefinitely.

Offline cached data is explicitly last-known authorization, not proof of current
membership. Neither offline cache nor permission revocation can erase data a user
already saw. SQLite is not encrypted; local OS file/account protection is required.
No E2EE, multi-device durable gateway guarantee or deployed acceptance is claimed.

Opening online verifies /auth/me and stores only the non-secret user profile in
binding metadata. On transport/429/5xx, an existing same-origin bound profile can
open offline. Before every reconcile/flush, /auth/me must still match the bound
account. Swapping credentials never transmits another account's queued content.
An initial 401 clears an existing cache as well. Unix cache creation uses 0600;
Windows protection follows the user's cache-directory ACL. WAL/shared-memory
files inherit SQLite protections; no untrusted shared cache directory is supported.

Dispatch sequences must increase, not be contiguous, because ephemeral typing is
intentionally omitted during replay. Resume verifies the same stream and a
nondecreasing Ready cursor; fresh Ready wakes a complete authorized reconciliation,
not unconditional trust in offline state. The old cache remains explicitly
last-known until that response completes. Only one worker can start per engine.
Ready has a five-second deadline, writes have five-second deadlines, HTTP and
WebSocket connects twenty seconds, frames 64 KiB and HTTP responses 32 MiB.
Outbox is bounded at 500 sends; cache history is disk-backed and currently has no
retention/eviction policy. Add one before sustained deployment. Access revocation
cascades through conversation/message/outbox rows. File deletion is not secure
erasure of SQLite pages, WALs, storage snapshots or backups.

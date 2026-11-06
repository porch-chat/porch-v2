# Messaging protocol

Phase 4 adds shared typed contracts in porch-protocol::messaging. All routes use
Bearer sessions, no-store responses and fresh permission checks in a transaction.
Community text channels get global conversation IDs; local channel IDs are not
interchangeable with them. Community rename/delete persists mapping atomically.
Voice channels never get a messaging conversation in this phase.

GET /conversations returns authorized ConversationHead objects including revision,
last nondeleted message, monotonic read cursor, unread and explicit mention counts.
Community heads require VIEW_CHANNEL + READ_HISTORY. Membership removal or hidden
channel loss removes the head. Clients MUST purge removed conversations locally.
Own messages and tombstones do not count as unread. Edits do not reset read cursors.
Explicit everyone mentions count for all other authorized readers.

POST /conversations accepts CreateDm, others' participant IDs and optional name.
One other user opens/reopens a unique unordered-pair 1:1 DM. Two or more peers
create a group; participants max 20 including actor. Existing usernames/discovery
is outside this API. Group membership mutations use DmCommand via POST
/conversations/{id}; only owner can rename/add/remove/transfer, anyone can leave.
1:1 membership cannot be changed. Group owner leave transfers to lowest remaining
ID; last leave deletes history. Added group members can see existing history.
There is no E2EE, block/consent flow or hide-1:1 operation yet. Owner moderation is
group-only; neither 1:1 participant may delete or edit the other's messages.

POST /conversations/{id}/messages accepts SendMessage. Markdown content is bounded
to 8000 UTF-8 bytes, nonblank and no NUL. Ciphertext is reserved and rejected 400.
nonce is 16..64 ASCII alphanumeric/hyphen/underscore, required for idempotency per
conversation+author. Repeating identical typed input returns the original current
message state, even if deleted; conflicting input returns 409. Fingerprints hash
canonical server-serialized typed requests, not client JSON bytes or key order.
Reply IDs must exist in that same conversation (also a composite DB foreign key).
Mentions are explicit max 20 unique IDs, must have channel visibility or be DM
participants. Text @names does not implicitly notify. Everyone requires its bit.
Sending requires SEND_MESSAGES + READ_HISTORY and visibility. Attachments later.

POST /conversations/{id}/messages/{message_id} accepts MessageCommand. Only author
edits (SEND_MESSAGES required), author or MANAGE_MESSAGES deletes, MANAGE_MESSAGES
pins, SEND_MESSAGES adds/removes one's own reaction. Max 50 pinned messages and
20 distinct reaction labels per message, label 1..64 UTF-8 bytes, no controls.
Reaction users are deduplicated and sorted. Deleted tombstones keep identity,
nonce/fingerprint, time and reply pointer but remove source, rendered HTML,
mentions, reactions and pin state. Search source/index are updated transactionally.
No edit history or raw deleted body is retained by this feature. Database backups
are not part of the tombstone guarantee. Deleting a channel/group deletes history.
No automatic expiry/retention policy exists yet.

GET /conversations/{id}/messages?before=ID&limit=50 gives newest-first keyset
history. GET .../pins uses the same paging. GET .../search?q=TEXT uses Postgres
simple-language FTS, not substring matching, same before/limit paging. Permission
is checked BEFORE querying a single conversation. No cross-conversation search,
counts or snippets leak. Search excludes tombstones. Limits are 1..100, query up
to 200 bytes. next is the last message ID when more pages exist; otherwise null.
Tombstones are included in history/deltas so clients can remove old content.

GET .../delta?after=REV&through=HIGH&limit=50 returns current changed messages
ordered by revision. First response captures high_watermark, and next is a revision
when more pages exist. Page with that same through value; after completion continue
from high_watermark in a fresh round. An update moving above the captured watermark
is retrieved in the next round. Each message has only its latest revision, not an
edit audit log. Membership changes also bump group revision, even without message
changes. Heads may change from renames/read states without a message revision.

POST .../read accepts MarkRead with an existing same-conversation message ID.
Cursors only move forward and read snapshots require READ_HISTORY. Updates notify
only that user's sessions. Message mutations publish payload-free SyncRequired
only to currently authorized readers; removals safely wake old+new DM members.
No content, resource IDs, mention IDs or secrets enter gateway retained replay.
Clients coalesce wakeups, reconcile heads, fetch changed deltas, periodically
reconcile after missed publication. No durable outbox/cross-node delivery yet.

Markdown source is stored as data; pulldown-cmark parses and ammonia sanitizes
HTML at write time, disallowing images, relative URLs and non-http/https/mailto
links. Native renderers may show plain source; NEVER interpret source as trusted
HTML. A future sanitizer security change must re-render stored messages. There
are no automatic remote previews/fetches, syntax highlighting or rich embeds yet.

Authenticated mutations share 120 requests/user/minute in memory. The 200-entry
map never evicts live counters; new users fail closed if full. Expired counters
prune after 60s. Restart resets counters and cross-process limiting is not claimed.
Request JSON is max 32KiB, persisted message JSON max 60,000 bytes, DB max 64KiB.
A user may participate in at most 100 DMs; create/add checks every affected user's
cap under a transaction advisory lock. Names use community validation. Page
validation errors 400, unauthorized access 404 for invisibility or 403 for missing
required bits, nonce conflicts 409, rate excess 429. Malformed JSON uses axum's
standard 4xx rejection. All feature HTTP responses carry no-store.

CLI commands are conversations, dm-create (PORCH_COMMAND), dm-mutate ID
(PORCH_COMMAND), messages ID, message-send ID (PORCH_MESSAGE), message-mutate ID
MESSAGE_ID (PORCH_COMMAND), message-read ID MESSAGE_ID, message-search ID QUERY,
message-pins ID and message-delta ID. PORCH_BEFORE/AFTER/THROUGH/LIMIT control pages.
Use credentials only through environment, not process arguments.

Local disposable PostgreSQL 16 measurement in debug build, 20,000 synthetic rows,
50 serial warmed full-HTTP FTS requests including authentication and authorization,
20 matching results each, p50 12.873ms / p95 14.664ms. Synthetic fixture, no concurrent
load, not a production SLA. Postgres FTS is adequate for this bounded app; revisit
with real dataset and concurrent load rather than adding another search service.

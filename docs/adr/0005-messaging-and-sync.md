# ADR-0005: Authorized message transactions and incremental sync
Status: accepted for Phase 4 implementation
Date: 2026-11-06

Use PostgreSQL FTS, not another search service at 200 users. A global conversation
ID maps community-local text channel IDs; standalone conversations implement
1:1 and group DMs. Metadata mapping is persisted transactionally alongside the
community aggregate. DM participant lists are bounded. Encryption is a tagged
content seam only; encrypted payloads fail closed until keys/protocol are designed.
No E2EE claim. No code reused from old applications.

Message mutation lock order is community (when present, FOR SHARE), conversation
(FOR UPDATE), then message. The shared community lock blocks authorization edits
without serializing unrelated channels. DMs never take community locks. Authorization and mutation share a
transaction; reads use a repeatable-read snapshot rather than a write lock.
No stale permission cache is trusted. This supersedes ADR-0004's proposed cache
requirement: the JSON aggregate at this scale is cheaper than a revocation hole.
Deletion of a community-local channel removes its conversation in the same
transaction. Owner/user deletion is not exposed in this phase.

Messages retain deleted tombstones and idempotency nonces. Retrying a deleted
send never resurrects it; conflicting content for one nonce fails. Replies are
constrained to the same conversation in the database. Tombstones expose no body,
mentions, reaction or pin state. IDs are keyset ordered within a conversation
under its write lock, not claimed to be globally commit ordered.

Each conversation has a transactionally incremented revision. Every changed
message, including edits, reactions, pin changes and tombstones, carries its
latest revision. A sync page has a captured high watermark; clients paginate
changes up to that watermark, then request another. This is current-state delta
sync, not an audit/event log. Membership revocation removes the conversation from
permission-filtered head listings and forces the client to purge its cache.

Gateway events remain payload-free wakeups, safe to replay after revocation.
Clients fetch small authorized conversation heads, then deltas only for changed
revisions, not full histories per message. Coalesce wakeups. Fresh connections
and periodic reconciliation recover commit/publication gaps. No durable outbox
or cross-node guarantee is claimed. Message content, message IDs and hidden
channel IDs never enter replay. Removed DM participants also get a safe wakeup.

Mentions use explicit validated IDs, never inferred from @text. Mention-everyone
requires the permission bit; raw source is not a notification. Markdown source
is data, not HTML. Rendering must use maintained parser plus sanitization, never
execute source HTML. Search authenticates before querying, is conversation-scoped
and excludes deleted bodies, with no unauthorized counts/snippets. Read cursors
are monotonic, validated against a visible same-conversation message.

Read-cursor writes also lock the conversation but do not bump its message revision;
private wakeups refresh head/read state. Reactions do bump revisions under the same
conversation lock as sends, so commit ordering cannot create skipped revisions.
Writes use READ COMMITTED; only read snapshots use REPEATABLE READ.

1:1 DMs have immutable participants and no moderation-owner privilege. Group DMs
allow the owner to moderate, rename, add/remove and transfer ownership. Owner
leave selects the lowest remaining user ID, and the last member leaving deletes
the conversation. A group may shrink to one member; it remains a group, never
changes into the unique-pair DM. Added members can read existing group history.
No blocking, per-recipient consent or DM hide operation exists in this phase.

No deployment or UI changes. Bounds, API, retention and measured search results
are in docs/messaging.md.

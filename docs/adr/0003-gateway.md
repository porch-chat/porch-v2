# ADR-0003: Ordered single-node gateway
Status: accepted for Phase 2
Date: 2026-11-06

No deployment or UI. GET /gateway upgrades using the same Authorization bearer
session as HTTP. Secrets never appear in URLs or Identify payloads. Browser
cookie/subprotocol authentication is not implemented. HTTPS/WSS remains required
outside loopback. Upgrade authentication checks Postgres; established connections
recheck revocation/expiry each heartbeat interval (15 seconds). Identify must
arrive within five seconds. Its version must equal PROTOCOL_VERSION (1).

porch-protocol owns all GatewayClient, GatewayServer and EventEnvelope types.
Hello advertises a client-initiated heartbeat every 15 seconds, acknowledged by
HeartbeatAck with the same nonce. Forty-five seconds without a client heartbeat
closes the connection at the next interval check. WebSocket ping/pong frames are
not application heartbeats. Clients use text JSON; malformed/binary input and
repeated Identify close the connection. Upgrade auth failures are HTTP 401.

Each identity session has one gateway stream, one active connection and its own
strictly increasing dispatch sequence. Simultaneous connections with the same
bearer get Conflict; distinct login sessions for one user aggregate presence.
Use separate identity sessions for separate devices. Stream IDs are random
process-local epochs, not credentials. Resume always requires valid bearer auth
and cannot resume another session's stream.

Identify includes an optional stream/sequence ResumeCursor. Ready says whether
resume succeeded. When resumed, dispatches newer than the cursor replay in order;
the Ready cursor precedes the pending replay, so persisting it cannot skip replay.
When fresh, Ready contains the current presence snapshot and baseline cursor.
Consumers persist a cursor after applying each dispatch, accept sequence gaps
(ephemeral events are intentionally not replayed), and discard old local state
on resumed=false. Replays hold the last 256 durable events per identity session.
Too-old/future/wrong-stream cursors, process restart and expired retention all
fall back to fresh sync rather than silently replaying an incomplete history.
Inactive streams expire opportunistically on the next attach after ten minutes.
There is no persistent or cross-node resume guarantee in this phase.

Presence aggregates DND above online above idle above offline across active
sessions. Disconnect/timeout removes that session's contribution, broadcasting
offline only if no other session contributes a higher status. Explicit offline
hides presence without disconnecting. Presence changes are limited to one per
second per connection. Typing is a member-only platform-lobby signal for now,
expires after five seconds and is limited to one per two seconds per connection.
It is never retained for replay. Arbitrary channel IDs/subscriptions are not
accepted before Phase 3 can authorize them.

EventBus::publish takes server-selected user recipients and a typed event.
Feature services will resolve permissions and subscriptions before publication;
untrusted clients cannot select recipients. The implementation assigns sequences
and retains replay under the same lock as attach/snapshot, preventing
subscribe/replay races. Notifications are registered before draining, preventing
lost wakeups. Queues/replay are bounded at 256 events, retained streams and
simultaneous upgrade sockets at 1024, text frames/messages at 4 KiB, outgoing
writes at five seconds and incoming commands at 60 per second. Slow consumers
are disconnected and must resume/fresh-sync; no silent overflow. The prior
connection retains its active lease until cleanup, including on overflow.

The bus is an internal dispatch seam, not a distributed implementation. Before a
second node, implement a broker-backed adapter with stream ownership, authorized
recipient routing, durable ordering/replay, shared presence and session routing.
No claim is made that today's in-memory buffers survive node failure.

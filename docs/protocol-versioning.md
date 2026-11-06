# Protocol versioning
porch-protocol is the sole owner of wire contracts. Current wire major is 1,
independent of Cargo package versions. Phase 0 defines only GET /health returning
{"status":"ok","protocol_version":1}. This is a scaffold, not a stable app API.

Before adding an endpoint/event, document its request, response, errors, authorization
and tests here or in a linked spec, then add shared types, server and CLI coverage.
Future API routes will be versioned under /v1; /health remains unversioned.
Gateway negotiation and rejection of incompatible majors must be implemented in
Phase 2, before accepting any realtime clients. No negotiation exists yet.

Additive optional fields and new opt-in capabilities may stay within the major.
Clients must ignore unknown object fields; new enum variants need explicit
compatibility handling, never an assumption that old clients can decode them.
Removing/renaming fields, changing meaning/types, or changing required behavior
requires a new major and an ADR describing the rollout and supported old versions.
Never silently reuse an event name for another meaning.

Persisted database schema versions are SQLx migration versions, not wire versions.
Every changed wire shape needs serialization/compatibility tests before implementation.
Authentication, errors, permissions and realtime envelopes are intentionally undefined
until their owning phases, rather than placeholder contracts that become accidental APIs.

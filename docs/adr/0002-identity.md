# ADR-0002: Identity and bearer sessions
Status: accepted for Phase 1
Date: 2026-11-06

Users are platform members, identities are authentication methods. A password
identity uses a normalized ASCII username as its provider subject. Later OAuth
providers can use issuer/subject and passkeys can use credential identifiers,
without adding password fields to users. There is no OAuth/passkey implementation
in this phase and no code from previous Porch repositories.

Usernames are case-insensitive ASCII letters, digits and underscore, 3-32 bytes.
Passwords are 12-128 UTF-8 bytes, never trimmed or case-folded. RustCrypto argon2
0.5.3 provides Argon2id v19 with random salts and its default parameters
(m=19456 KiB, t=2, p=1). Work runs off the async executor with at most four
concurrent hash/verify operations. Unknown valid usernames verify a dummy hash
to reduce account enumeration through timing. Duplicate registration returns
conflict and therefore does not conceal username availability.

Session credentials and invite codes have 256 bits of OS randomness, encoded
as hex. Only SHA-256 digests are stored. Unsalted SHA-256 is intentional for
uniform high-entropy credentials, not passwords. Bearer tokens are returned once
and expire after 30 days; revocation and user flags are checked from Postgres
on every request. No refresh tokens, cookies, password reset or browser login
are implemented. Headers must contain Authorization: Bearer TOKEN. Never put
tokens in URLs. Auth responses set Cache-Control: no-store. No CORS policy
enables browser callers. TLS termination is a deployment prerequisite.

Invite creation requires admin or can_invite. A user can see/revoke their own
invites even if their creation privilege is removed; admins see/revoke all.
Invites last 60 seconds to 30 days, permit 1-200 registrations, and do not grant
platform roles. Guarded SQL updates consume uses inside the user/identity/session
transaction; any subsequent failure rolls everything back. A transaction-scoped
advisory lock serializes the 200-member cap and one-time bootstrap. No endpoint
exists to bypass invite registration, promote users or bootstrap an admin.

Bootstrap is a one-shot local server subcommand using environment credentials.
It only succeeds when the users table is empty, and never resets/upgrades an
existing account. Platform role administration is reserved for Phase 10; this
phase exposes flags in the shared User contract and enforces them.

The single-process auth limiter allows 30 combined login/register/invite-create
attempts per 60-second fixed window, including failed valid-JSON attempts.
A separate four-slot semaphore prevents hash overload. This is deliberately a
bounded, coarse global budget, with no unbounded attacker-keyed map or trust
in X-Forwarded-For. It can cause shared denial of service and resets on restart.
Before internet deployment, add proxy ingress limits, trusted-peer configuration
and account/source budgets; before multiple replicas, use a shared limiter.
Other authenticated read/revoke endpoints do not run expensive password hashes.
JSON bodies are bounded to 4 KiB. Secrets are omitted from Debug implementations.

Postgres errors are logged internally, never returned verbatim over HTTP.
Framework extraction errors (malformed JSON, unsupported content type, oversized
body, invalid numeric path) retain axum's built-in status/body, not ErrorResponse.
Database rows never directly serialize; porch-protocol owns API shapes.
No session/invite pagination or retention job yet; add these before opening
public registration or a sustained deployment.

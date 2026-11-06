# Phase 1 identity runbook

No deployment or UI is included. Use disposable Postgres for all proof commands.
Migrations run before binding. Local development defaults to 127.0.0.1:38147;
override PORCH_BIND and DATABASE_URL. Do not commit credentials.

## Bootstrap
With DATABASE_URL set, export PORCH_BOOTSTRAP_USERNAME and
PORCH_BOOTSTRAP_PASSWORD from your secret store, then run:

    cargo run -p porch-server -- bootstrap-admin

Unset the bootstrap variables after success. Repeating this command refuses
instead of changing any account. Start normally with:

    cargo run -p porch-server

## CLI
PORCH_URL defaults to http://127.0.0.1:38147. Non-loopback origins require HTTPS.
Redirects are refused so credentials cannot be forwarded to another origin.
Password/token/invite values come from environment variables, not arguments.

Export PORCH_USERNAME and PORCH_PASSWORD, then:

    cargo run -p porch-cli -- login

The returned JSON contains a session token. Store it securely and set PORCH_TOKEN
for subsequent commands. CLI JSON intentionally prints one-time credentials for
automation; never paste its output into logs, issue trackers or shell history.

    porch-cli me
    porch-cli sessions
    porch-cli invite-create 2 3600
    porch-cli invites
    porch-cli invite-revoke INVITE_ID
    porch-cli revoke-session SESSION_ID
    porch-cli logout
    porch-cli logout-all

To register, export PORCH_USERNAME, PORCH_PASSWORD and PORCH_INVITE (the one-time
code returned by invite-create), then run porch-cli register. New accounts receive
neither platform flag. Revoke-session only affects your own sessions; logout-all
revokes every active session for the current user, including the caller.

## Verification
Run these before every commit:

    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
    cargo build --workspace --locked

With DATABASE_URL pointing at a disposable database:

    cargo test -p porch-server --locked postgres_migrations -- --ignored
    cargo test -p porch-server --locked --test identity -- --ignored
    python3 scripts/identity-smoke.py

The integration test creates/removes an isolated schema. The CLI smoke script
requires an empty users table and deliberately creates test members in the
configured database. Do not run it against any real user database. CI provisions
ephemeral Postgres 17 and runs all three proofs explicitly, not just skipped tests.
The script starts a temporary loopback HTTP process, exercises real CLI commands,
and stops the process in a finally block. It does not deploy any service.

## Routes
POST /auth/register takes RegisterRequest and returns 201 AuthResponse.
POST /auth/login takes LoginRequest and returns AuthResponse.
GET /auth/me returns User.
GET /auth/sessions returns active Session records with a current flag.
DELETE /auth/sessions revokes all sessions.
DELETE /auth/sessions/{id} revokes an owned session.
POST /auth/logout revokes the current session.
POST /invites takes CreateInviteRequest and returns 201 CreatedInvite.
GET /invites returns owned invites, or all for admins.
DELETE /invites/{id} revokes an owned invite, or any for admins.
Revocation/logout success returns 204. Missing or unowned IDs return 404.
Application errors return ErrorResponse; see ADR-0002 for framework rejections,
limits, security assumptions and deliberate deferred work.

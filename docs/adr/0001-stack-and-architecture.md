# ADR-0001: Rust workspace and shared contracts
Status: accepted for Phase 0
Date: 2026-11-06

Porch is a private, clean-slate system for at most 200 users. No old Porch code is reused.

Use a Cargo workspace with porch-protocol (serializable wire contracts), porch-server
(axum/Tokio HTTP and future gateway, SQLx/Postgres), porch-core (shared client engine)
and porch-cli (backend proof without UI). Rust 1.99.1 and Cargo.lock are pinned.
GPUI, web and mobile consumers are deferred. No UI framework is added in Phase 0.

Environment variables configure the process; JSON tracing goes to stdout and RUST_LOG
controls filtering. Database connection and embedded SQLx migrations must succeed
before binding HTTP. /health is process liveness, not database readiness after startup.
Metrics instrumentation/export is deferred; structured logging is available now.

Database migrations are append-only after publication. CI applies them twice to a
disposable Postgres database to check repeatability. Phase 0 has only a no-op baseline,
not premature identity tables. Pool size is bounded to five connections for now.

The plan's staging deployment is explicitly deferred by Phase 0's execution scope.
deploy/ contains an unlaunched Compose/Docker skeleton. No dedicated server, Caddy,
DNS, LiveKit or other infrastructure is changed. Production backups, secret delivery,
readiness monitoring and image hardening require a deployment ADR before launch.

Alternatives: separate client stacks duplicate state/sync; reusing legacy forks carries
unwanted code and licensing constraints. A shared Rust core keeps later native clients
on the same contract without adding UI dependencies to the server.

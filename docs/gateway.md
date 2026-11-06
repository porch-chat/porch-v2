# Gateway testing and CLI

Start the server using the normal DATABASE_URL and PORCH_BIND configuration.
No separate gateway process or deployment is needed.

    PORCH_URL=http://127.0.0.1:38147 PORCH_TOKEN=... cargo run -p porch-cli -- gateway

The command connects/listens, prints shared protocol JSON and acknowledges
liveness through periodic client heartbeats. It exits nonzero on disconnect;
automatic reconnect belongs to porch-core in Phase 5. To resume manually, set
both PORCH_STREAM and PORCH_SEQUENCE to the last applied dispatch cursor.
Ready carries the initial stream ID. Credentials remain in environment/header.

Run integration tests with a disposable Postgres database (each test creates
and drops an isolated schema):

    cargo test -p porch-server --locked --test gateway gateway_integration -- --ignored
    cargo test -p porch-server --locked --test gateway gateway_load_200 -- --ignored --nocapture

CI explicitly runs the gateway integration and actual CLI gateway smoke, plus
normal unit tests, identity integration, migration checks, fmt, clippy and build.
The 200-client load test is manual/ignored so CI isn't a performance gate.

## Load result, sandbox loopback, 2026-11-06

Four Tokio worker threads, unoptimized debug test build, real PostgreSQL auth,
200 distinct users and bearer sessions, 200 simultaneously open real WebSockets.
Each client performs 20 nonce/ack rounds: 4,000 heartbeat RTT samples total.
All clients completed with no rejected connection or failed sample.

Heartbeat RTT in milliseconds: p50 3.307, p95 12.524, p99 18.091, max 27.479.
One 200-recipient event-bus fanout: p50 81.234, p95 111.642,
p99 112.648, max 113.344 milliseconds across 200 deliveries.

Fanout is a cold initial burst, deliberately including drainage of presence
events accumulated while 200 clients connect and synchronize at a barrier.
It is not steady-state per-message latency. Timing excludes account/session
fixture insertion and includes actual JSON framing/socket transport. These
numbers are local debug evidence, not internet/TLS production guarantees.
See ADR-0003 for wire behavior, bounds, retention and future-node prerequisites.

# Porch v2
Private, clean-slate Rust foundations. Phase 0 only, no deployment or UI.

Read docs/PLAN.md, docs/adr/0001-stack-and-architecture.md and
docs/protocol-versioning.md before adding features.

## Local development
Install Docker Compose and Rust (rust-toolchain.toml pins the toolchain).
Run these commands from the repository root:

```sh
docker compose up -d --wait
cp .env.example .env
set -a; . ./.env; set +a
cargo run -p porch-server
# In another shell:
curl --fail http://127.0.0.1:38147/health
cargo run -p porch-cli
```

.env is not automatically loaded by the app; export it as shown.
Local Postgres is bound only to loopback on 39463. Example credentials are for
disposable local data only. Stop with docker compose down (data volume is retained).
The CLI currently prints the shared protocol version; it is not a network client.

## Quality gate
```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test -p porch-server --locked postgres_migrations -- --ignored
cargo build --workspace --locked
```
The migration test requires exported DATABASE_URL pointing at a disposable Postgres.
GitHub CI provides Postgres and runs the same checks including that integration test.

## Migrations
Migrations live in migrations/ and are embedded in the server binary. Startup applies
pending migrations before opening HTTP. Never edit an already published migration.
For new migrations use a unique timestamp filename and review the SQL. Optionally
install sqlx-cli 0.8.6 with Postgres support and use sqlx migrate add <name>.
The baseline creates no domain tables; SQLx creates its own migration bookkeeping.

## Deployment
deploy/compose.yaml and deploy/Dockerfile are skeletons, not an approved deployment.
They have not been launched or container-built. No dedicated server, DNS or proxy was
changed. Secrets, backups, readiness checks and a production rollout need approval.

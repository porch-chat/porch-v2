# Porch build rules
Read docs/PLAN.md and applicable ADRs before editing.
No code from old Porch repositories. Contracts before implementation; backend and CLI tests before UI.
porch-protocol owns wire types; clients never invent them.
Keep main green. Before every commit and push run:
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
Use small atomic commits, authored Dylan <dylan@dylanh.dev>.
Never commit credentials. Configuration comes from environment variables.
No infrastructure changes or deployment without separate approval.
Phase 0 excludes GPUI, auth, gateway, messaging and UI.

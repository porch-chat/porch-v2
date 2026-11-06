# Implementation progress

## 2026-11-06

Phase 0 foundations, Phase 1 identity and Phase 2 gateway are complete with green
GitHub-hosted CI. Phase 3 is complete in the same worker, no deployments.

CI speedup commit 2e5f1ae retains every ignored DB test serially, CLI smoke and
single hosted job. Baseline run 42173780003 took 157s workflow / 154s job without
Rust caching. Optimized cold run 42175402463 attempt 1 took 274s workflow (cache
population and nextest compile mode made cold slower). Warm attempt 2 succeeded
in 81s workflow / 76s job. No self-hosted runners or global git config changes.

Phase 3 protocol/permissions commit f906231 and persistence/authorization commit
0bde930 are locally green. CLI lifecycle smoke and integration coverage added.
Community defaults, category-sync semantics, bounded aggregate row locks,
idempotent official seeding, invitation races and payload-free replay-safe
invalidations are documented in ADR-0004 and communities.md. Phase 3 workspace/DB/CLI checks passed, 21 default tests and 5 ignored DB tests.
Hosted CI 42176668393 at 6708ce8 succeeded in 111s workflow / 92s job.
Exact Phase 3 commits are f906231 (contracts/engine), 0bde930 (persistence/HTTP),
54b8ddb (CLI/tests/docs). Green run https://github.com/porch-chat/porch-v2/actions/runs/42176668393
also includes branding 6708ce8. The prior phase run was cancelled during cache
upload by branding after its test steps passed, so only the replacement is called
green. No graphical, deployed or live-user acceptance has occurred.
Phase 4 messaging has started, design in ADR-0005.

Dylan explicitly approved sequential continuation through later phases, with
local tests and green hosted CI before each next phase. No extra workers, no PC
access, no dedicated-server deployments, DNS, Caddy or infrastructure changes.
Deployment-dependent live validation must be marked blocked, never implied.

For Phase 6, design inspiration only is the familiar Discord-style server rail,
channel sidebar, message pane and member list (Fluxer/Sharkord are UX references,
not code sources). Porch should feel warm and close-knit, with its own character.
Read assets/brand/BRAND.md and use the concurrent branding assets when present;
never replace branding task files. Load frontend-design before UI coding, write
a design ADR, build Linux/Windows artifacts or honestly document build blockers.
Headless screenshots where feasible; graphical acceptance belongs to Dylan alone.
Before every push, safely commit local work then pull --rebase to preserve branding.

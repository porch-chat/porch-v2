# Implementation progress

## 2026-11-06

Phase 0 foundations, Phase 1 identity and Phase 2 gateway are complete with green
GitHub-hosted CI. Phase 3 is being completed in the same worker, no deployments.

CI speedup commit 2e5f1ae retains every ignored DB test serially, CLI smoke and
single hosted job. Baseline run 42173780003 took 157s workflow / 154s job without
Rust caching. Optimized cold run 42175402463 attempt 1 took 274s workflow (cache
population and nextest compile mode made cold slower). Warm attempt 2 succeeded
in 81s workflow / 76s job. No self-hosted runners or global git config changes.

Phase 3 protocol/permissions commit f906231 and persistence/authorization commit
0bde930 are locally green. CLI lifecycle smoke and integration coverage added.
Community defaults, category-sync semantics, bounded aggregate row locks,
idempotent official seeding, invitation races and payload-free replay-safe
invalidations are documented in ADR-0004 and communities.md. Phase 3 final
workspace/DB/CLI checks and hosted CI are the next gate before Phase 4.

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

# ADR-0004: Bounded community aggregates and safe invalidation
Status: accepted for Phase 3
Date: 2026-11-06

Communities use a versioned typed JSONB aggregate, bounded to 200 members, 128
roles, 128 categories, 512 channels and 200 bans. This is an intentional small
platform tradeoff, not an unbounded document store. Postgres stores owner and
membership/ban projections with foreign keys and unique constraints; invite
credentials are separate constrained rows. The typed aggregate is authoritative.
All projections are updated in the same transaction. One community row lock
covers authorization, mutation, invariant validation and save; no authorization
read is reused across transactions. Reads build explicit permission-filtered DTOs
from one consistent aggregate snapshot. Schema version changes require migrations.
No code is carried from prior repositories.

Creation/official seeding use a transaction advisory lock for platform caps and
idempotence. Invites lock community before invite. Join first looks up the hash
to find community ID without a lock, locks community, then revalidates the invite
with a guarded update. Codes contain 256 random bits, only SHA-256 is persisted.
Invites have 1-200 uses and 60 seconds to 30 days expiry, and never grant roles.

Permission evaluation is owner/admin bypass, base everyone plus member roles,
everyone overwrite, aggregate role denies then allows, member overwrite, then
implicit view/send/connect restrictions. Administrator is not a channel override.
Hierarchy still protects equal/higher roles and the owner even from administrators.
Role grants must be a subset of actor's possessed base bits. Overwrite changes
require effective channel/category permissions and cannot affect bits actor lacks.
Everyone identity/name/order is fixed; its permission mask may be edited.
Category-synced channels dynamically resolve category overwrites; explicit channels
have independent overwrites. Unsync starts with no local overwrites. Category
removal refuses nonempty categories. Ordering is dense and bounded, roles higher
position are stronger, channel order is per parent.

Phase 3 publishes payload-free SyncRequired invalidations through EventBus only
to old/new affected members. No community IDs, hidden channel IDs, snapshots,
permissions, invite secrets or moderator payloads enter replay. This deliberately
avoids historical privileged-data replay after revocation: fresh and resumed
clients refetch filtered GET /communities then individual snapshots, removing
cached state on missing membership/visibility. Fresh gateway Ready already tells
clients to discard old state. Invalidation is idempotent and safe to replay after
any permission change, leave, kick or ban. Capture-bus and real gateway tests cover
this contract. An event lost in the DB-commit/publication crash window requires
fresh sync, as in Phase 2; no durable transactional outbox is claimed. Message
payload routing/replay authorization is a separate Phase 4 obligation, not solved
by these metadata invalidations. Message hot paths must not refetch the aggregate
per message; add an authorization projection/cache with explicit invalidation
before messaging. No UI, broker, deployment or infrastructure changes.

The official Porch community is seeded idempotently with the earliest platform
administrator as protected owner and existing/new platform users as read-only
members by default. It cannot be deleted or transferred through ordinary commands.
Its owner can deliberately delegate roles. Ordinary communities allow creation
by any authenticated user, at most 10 owned per user and 200 communities total.
Future scale beyond these bounds requires normalized query projections and a new
ADR, not silently removing limits. HTTP request bodies are bounded and domain
validation rejects invalid IDs, names, masks, dangling references and ordering.

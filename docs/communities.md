# Communities and authorization

All endpoints require an Authorization bearer session. Platform invites at /invites
are distinct from community invitations. Platform admin is not community admin.
No list, snapshot, gateway event or invite list returns invitation secrets.
Every mutation commits before publishing a payload-free sync_required gateway event.
Clients refetch /communities and then visible snapshots, replacing cached resources,
not merging vanished channels back in. Missing membership returns 404.

GET /communities returns current membership summaries only.
POST /communities takes {"name":"Friends"}; any authenticated user may create.
GET /communities/{id} returns a filtered CommunitySnapshot, including visible text
and voice channels, categories containing visible channels (or visible empty
categories), roles, members and base permissions. Bans require BAN_MEMBERS.
POST /communities/{id} takes a tagged CommunityCommand. See porch-protocol's
community.rs for authoritative fields and bit constants. No hidden channels or
hidden-only categories are serialized in snapshots.

CommunityCommand operations include rename, transfer_ownership, delete,
create_role/edit_role/delete_role, set_member_roles,
create_category/edit_category/delete_category,
create_channel/edit_channel/delete_channel, set_overwrites, leave, kick, ban, unban.
Roles have dense global positions with @everyone at zero and higher positions
stronger. Categories have dense positions; channel positions are per category or
uncategorized. Moves validate the final destination index. Nonempty category
removal fails with conflict. Role deletion cleans assignments and overwrites.
Member removal cleans member overwrites. Members cannot modify equal/higher
roles or users, even with administrator. The owner cannot leave, be kicked/banned
or have roles assigned by another actor; transfer ownership or delete first.
Category-synced channels inherit dynamically. Unsynced channels use explicit
local overwrites only. SetOverwrites targets exactly one channel or category;
a synced channel must be unsynced before setting local overwrites.

POST /communities/{id}/invites accepts the same bounded CreateInviteRequest
shape as platform invites, but returns CreatedCommunityInvite with a code once.
GET lists own invites, or all for MANAGE_COMMUNITY, requiring CREATE_INVITE.
DELETE /communities/{id}/invites/{invite_id} revokes own invites or manager-owned
scope. POST /communities/join takes {"code":"..."}; it never consumes platform
invites. Joins do not grant roles. Already joined returns conflict without using
an invite use. Codes expire, are revocable, limited-use, hashed and race-safe.
Invite records are capped at 500 per community, including expired/revoked rows;
this conservative lifetime cap prevents unbounded storage. Future pruning is a
separate policy decision. Communities cap 200 total, 10 ordinary owned per user.

CLI examples, with PORCH_URL and PORCH_TOKEN set in the environment:

    porch-cli communities
    porch-cli community-create 'Friends'
    porch-cli community 2
    porch-cli community-invite-create 2 10 3600
    PORCH_COMMUNITY_INVITE=... porch-cli community-join
    PORCH_COMMAND='{"op":"create_role","name":"Regular","permissions":4095,"position":1}' porch-cli community-mutate 2
    PORCH_COMMAND='{"op":"create_channel","name":"general","kind":"text","category_id":null,"position":0,"synced":false}' porch-cli community-mutate 2
    PORCH_COMMAND='{"op":"leave"}' porch-cli community-mutate 2
    porch-cli community-invites 2
    porch-cli community-invite-revoke 2 1

JSON command mode round-trips shared typed contracts, rejecting unknown fields;
it does not invent a separate CLI schema. Names are 1-100 UTF-8 bytes, not all
whitespace, with no control characters. Request bodies cap 64 KiB.

Bootstrap and server initialization idempotently seed Porch Announcements with
the earliest platform admin as protected owner, a text announcements channel,
and read-only @everyone. Existing users are initialized once; registration joins
new users in the same transaction. Voluntary leaves and bans are not undone on
restart. No standalone seed/deploy operation is needed. The official community
cannot be deleted or transferred by ordinary commands.

Tests include 512-case permission properties, role/member hierarchy and owner
protection, implicit permissions, category synchronization and hidden snapshots,
real PostgreSQL HTTP integration, concurrent last-use invites and concurrent
mutations, isolated EventBus audiences, offline safe invalidation replay and an
actual CLI smoke spanning the community lifecycle. Run all ignored DB tests
serially against a disposable DATABASE_URL; migrations are tested for idempotence
and expected versions. scripts/identity-smoke.py expects an otherwise empty DB.
See ADR-0004 for aggregate bounds, lock order, outbox limitations and Phase 4 work.

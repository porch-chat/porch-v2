# Phase 8 isolated dedicated-server runbook

Deployment explicitly authorized by Dylan on 2026-11-06. No DNS changes authorized.
Target debian@40.160.53.64, SSH key ~/.ssh/kat_vps. New directory
/home/debian/porch-v2-phase8, Compose project porch-v2-phase8 only. Do not reuse
old fluxer LiveKit, databases, volumes, networks, ports or credentials.

Preflight read-only inventory on 2026-11-06 found 74 GiB disk available and
38 GiB available RAM. 38147/tcp, 39780/tcp, 39781/tcp and 39782/udp unused.
Recheck before starting. Server binds loopback 38147; Postgres no host port;
LiveKit signaling binds loopback 39780, ICE public 39781/tcp + 39782/udp.
Use explicit advertised IPv4 40.160.53.64. No broad UDP range or TURN initially.
Restricted networks needing TURN are a known untested boundary, not a promise.

Before action commit reviewed compose and this runbook. Copy exact green source
archive, build server image only, pin LiveKit and Postgres images by digest after
pulling. Generate random URL-safe DB password/key/secret on host, chmod 600 .env
and livekit.yaml; never print or commit credentials. Set resource/log caps and
restart policies for these three new containers. Keep file storage private with
UID 10001 ownership; do not enable FFmpeg or outbound embeds.

Start Postgres + LiveKit, verify DB health and authenticated SFU room RPC; start
Porch after migrations. Bootstrap administrator through environment, not argv,
and remove the one-time bootstrap secret afterward. Verify loopback health,
real login/community/voice HTTP, token grants and unauthorized denials. Compare
existing running container IDs before/after, with no restarts outside this project.
Back up every preexisting config before editing it; none should need editing for
loopback deployment. Record source SHA, image digests and smoke output without
secrets under this new directory.

Public HTTPS/WSS requires an approved hostname. Proposed A records api.porch.chat
and voice.porch.chat to 40.160.53.64 (voice DNS-only, not CDN proxied). Ask Dylan
before DNS; do not guess domain ownership. Public TLS/proxy route addition must be
backed up and validated, never overwrite an existing route. Until approved,
loopback service plus SSH forwarding can validate control plane but is NOT a
publicly usable desktop voice deployment. Do not send plaintext public tokens.

Rollback script must execute docker compose -p porch-v2-phase8 down without -v,
retaining PostgreSQL/files/secrets/evidence and leaving all existing services alone.
Do not prune Docker or delete any data. Fresh deployments can simply stop these
three containers. Later upgrades require pg_dump first and a prior immutable
server image tag; do not assume schema downgrade safety.

## Execution record, 2026-11-06

Backend commit 11a4cfb deployed after all four local workspace checks and the
explicit disposable-Postgres voice test passed. New compose directory/project as
above, rollback.sh present and executable, persistent data retained. Existing
container ID/name inventory comparison passed unchanged. No shared config edited.
Runtime uses an Ubuntu 24.04 image around the exact locally checked debug server
binary (glibc-compatible); source-build bookworm skeleton was not used. Runtime
image sha256:56ade0b86cf36a6a05ecabb214f468639b0f7e6232571b56894964f406d251ee.
Postgres pinned sha256:665dc7b37ac52a17e298511414c21d23ffa4e6a6ef6225b3e5d59c7afb35e994.
LiveKit 1.13.11 pinned sha256:25cd902de7cacbd5f1f779f6a6ac1eae20ba8f56b0935934ce6b0ab80d1e3d20.
Real HTTP smoke passed login, community/channel creation, real LiveKit room RPC,
ticket minting, flags/deafen normalization, authorized snapshot, leave and logout.
This is control-plane evidence, NOT successful microphone/audio transport proof.
Administrator username dylan; generated password is host-only admin-password mode
0600, not in source, logs, CLI args or this document. LiveKit config root-owned 0600
because all capabilities are dropped. Native hardware acceptance remains pending.
LiveKit warns about host UDP socket buffer size; no global sysctl was changed.

Dylan approved the two DNS records. api.porch.chat already pointed to
40.160.53.64 (record 6a556647f612953c68a1bb98eaf6c1b2, TTL 60, DNS-only),
so it was not changed. voice.porch.chat A 40.160.53.64 was added DNS-only TTL
300 (record 51abb4d58f0b706651838329f6048890). No other DNS records changed.
The API hostname is already claimed by existing fluxer-caddy-1 Traefik router
porch-api. Do not silently override it. Requested separate approval to repoint that
route or choose v2-api.porch.chat. Until resolved, public API attachment is blocked.

# Porch v2 Plan (draft for Dylan's audit)

## What Porch is
A close-knit, invite-only chat app for friends, family and friends of friends. Discord-style communities anyone can create, plus one official Porch community for announcements. 50-100 users realistic, 200 hard max. Private code, built by Kat, audited by Dylan. No deadline. No old code reused.

## Core principles
1. **Contracts before code.** The protocol (API + realtime events) is defined and versioned first. Every client is just a consumer of it.
2. **Backend before UI, always.** A feature is "done" on the server, with tests and a CLI client proving it, before any UI touches it. UI never invents data shapes.
3. **Shared Rust core.** Networking, sync, cache and state live in one Rust crate (porch-core). The GPUI desktop app uses it directly; iOS/Android later wrap it via UniFFI. Logic is written once.
4. **Fast by design.** One persistent WebSocket per client, hot state in server memory, optimistic UI (messages appear before the server confirms), local cache so the app opens instantly.
5. **Built for later.** Things we're not building yet (OAuth, passkeys, E2EE DMs, video, bots, R2, a fallback node) get a seam in the design now so they're additions, not rewrites.
6. **Main stays green.** Kat pushes to main with summaries, so CI (fmt, clippy, tests, integration tests) must pass before every push.

## Stack
Server: Rust, axum, tokio, WebSocket gateway, sqlx + Postgres, tantivy or Postgres FTS for search.
Files: dedi disk behind a Storage trait (R2 later is a config swap). 1GB max, chunked resumable uploads.
Voice: self-hosted LiveKit behind our own voice API (swappable).
Desktop: GPUI on Windows + Linux.
Web/PWA: separate lightweight web client (TS) on the same protocol, for phones until native apps.
Mobile later: SwiftUI iOS app on porch-core, then Android.
Hosting: OVH dedi behind Caddy. Postgres ready for a streaming replica (fallback node) later.

## Repo layout (one private monorepo, porch-chat/porch-v2)
crates/porch-protocol  — every type, event, error and permission bit. Single source of truth.
crates/porch-server    — API, gateway, auth, permissions, storage.
crates/porch-core      — client engine: connection, reconnect/resume, cache, sync, state.
crates/porch-cli       — tiny terminal client for testing everything without UI.
apps/desktop           — GPUI app.
apps/web               — PWA (later phase).
deploy/                — docker compose, Caddy, LiveKit, backups.
docs/                  — protocol spec, decisions log (ADRs).

## Build order
Each phase ends with something that *works end to end* and is tested, before moving on.

**Phase 0, Foundations.** Monorepo, CI, Postgres migrations setup, config, logging/metrics, deploy skeleton on the dedi (staging subdomain). Protocol versioning rules + ADR log.

**Phase 1, Identity.** Users table separate from identities table (so OAuth/passkeys slot in later). Username+password (argon2), sessions/tokens, invite codes, platform roles (admin, can-invite). Proven via CLI.

**Phase 2, Gateway + realtime.** WebSocket gateway with heartbeat, resume after disconnect (replay missed events by sequence number), presence, typing. Load test with 200 fake clients.

**Phase 3, Communities + permissions.** Communities, categories, channels, roles, Discord-style permission bitflags with channel overrides, member list, community invites. The permission engine gets heavy unit tests because everything sits on it.

**Phase 4, Messaging.** Send/edit/delete, replies, mentions, reactions, pins, markdown, read states, unread counts, history pagination, full-text search. DMs and group DMs (just channels with no community). Message model leaves room for encrypted DMs.

**Phase 5, porch-core.** Client engine: local cache (SQLite), optimistic sends, reconnect/resume, sync. Driven by the CLI first so it's proven before any UI exists.

**Phase 6, Desktop app v1.** GPUI app on porch-core: login, community/channel list, message view, composer, DMs, settings. Windows + Linux builds. *First time Dylan clicks anything.*

**Phase 7, Files + embeds.** Chunked resumable uploads to 1GB, image/video previews, link embeds (server-side fetching with safety limits).

**Phase 8, Voice.** LiveKit deploy, voice channels, join/leave/mute/deafen, device selection, push-to-talk, speaking indicators. Real hardware testing early.

**Phase 9, Threads + notifications.** Threads, notification settings per channel/community, native desktop notifications, web push groundwork.

**Phase 10, Platform admin.** Admin panel: global bans, reports, user management, invite permissions, announcements community.

**Phase 11, Updates + channels.** Single client with in-app stable/canary switch. Background download, prompt to restart, auto-apply on launch. Live-pushable feature flags, config, themes and text without restart. Signed releases.

**== Launch (friends & family) ==**

**Later.** PWA (with iOS push caveats), OAuth/Google linking, passkeys, 2FA, video + screenshare (capture cards), bots/webhooks, R2, fallback node, native iOS ($99/yr) then Android ($25 once), opt-in E2EE DMs, branding refresh.

## Lessons from old Porch (no code carried over)
Session recovery and reconnect must be first-class, not patched in. Device routing (audio in/out, capture cards) needs real hardware tests early. The updater needs correctness tests. Building UI before backend caused constant rework.

## Open decisions
Branding (keep or redo). Exact web client framework (decide at PWA phase). Search engine (Postgres FTS vs tantivy, decide in Phase 4 with benchmarks).

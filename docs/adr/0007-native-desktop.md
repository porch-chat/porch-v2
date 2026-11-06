# ADR-0007: Evening Threshold native desktop
Status: accepted for Phase 6 implementation
Date: 2026-11-06

Use published GPUI 0.2.2 + gpui-component 0.5.1, the compatible pair in the
component package's README. Apache-2.0 libraries supply platform rendering and
maintained text editing/IME, selection, clipboard and focus rather than custom
input code. chub has no GPUI entry; use versioned downloaded crate source/docs.
Do not copy old Porch/AGPL code. The existing brand assets are original artwork.

Warm architectural minimalism: near-black pine, cream text, amber actions and
Manrope, embedded from the approved OFL font. Fixed community rail, categorized
channel sidebar, bounded 200-message page, composer and optional member column.
Use the approved porch icon intact. No decorative gradients or online dots
pretending membership is presence. User-generated Markdown is displayed as plain
source text, not HTML, and never loads remote images/links automatically. Rich
Markdown, screen-reader verification and virtualized unbounded history are not
claimed. Message history is deliberately bounded; paging is explicit.

A dedicated Tokio worker owns porch-core Engine/GatewayTask. GPUI only sends
bounded commands and reads immutable snapshots; it never performs network/SQLite
work on the render thread. The existing core owns cache, nonce, authorization,
reconnect and sync. Coalesced notifications avoid an unbounded UI queue.
Only durable queue acknowledgment clears the submitted composer text. Local
drafts are conversation-bound and cleared at session generation boundaries.
Edits retain source until successful acknowledgment, and failed edits stay edits.

Session tokens/passwords are memory-only, not preferences, command-line flags,
logs or screenshots. Login each launch is a deliberate v1 boundary until an OS
credential-store design exists. Environment token login is development-only.
Settings contain server address, font size and member-column visibility, never
secrets. Cache is plaintext in a user-private directory, not E2EE. Sign-out asks
for confirmation and warns that purged local sends may have reached the server.
Removal notices survive relogin, and queue discard never claims remote deletion.

Linux builds/tests use local disposable Postgres and optional Xvfb/Mesa captures.
Windows builds run on a GitHub-hosted Windows runner, not Dylan's PC or a
self-hosted runner. Artifact compile success is NOT Windows runtime, IME,
clipboard, DPI, graphics-driver or accessibility acceptance. Dylan owns all
graphical user acceptance. No dedicated-server/infrastructure deployment.

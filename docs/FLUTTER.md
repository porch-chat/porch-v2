# Flutter vertical slice

Flutter is the chosen long-term client. GPUI stays buildable. See ADR0011.

## Build and generation

Use Flutter 3.41.8 and the repository-pinned Rust toolchain. Linux requires clang, CMake, Ninja, GTK3 and libpulse development packages. Windows requires Visual Studio Desktop development with C++ and CMake.

From the repository root run scripts/flutter-build.sh on Linux or scripts/flutter-build.ps1 in PowerShell. Linux output is apps/porch_flutter/build/linux/x64/release/bundle/ and Windows is apps/porch_flutter/build/windows/x64/runner/Release/. Launch porch_flutter or porch_flutter.exe. Keep the ENTIRE bundle folder, including Flutter/WebRTC libraries, data and the Rust adapter. Artifacts may wrap the folder under build/ because a font license is included too. Rust currently uses the debug profile while Flutter is release; this is not an optimized Rust performance claim.

Install flutter_rust_bridge_codegen exactly 2.11.1 with cargo install --locked, then run flutter_rust_bridge_codegen generate --config-file flutter_rust_bridge.yaml at the repo root. Runtime and generator versions must match. Commit generated Rust and Dart together; do not edit generated bindings manually.

## Boundaries

The adapter owns one process session and one subscriber. Async FFI commands use porch-core; a 500ms snapshot stream projects its authoritative cache. Auth, account/origin-bound SQLite, nonce allocation, durable enqueue, reconciliation, permission checks and actual gateway WebSocket stay in Rust. Dart never duplicates REST/WebSocket networking; the official LiveKit SDK handles media transport only. Rust obtains short-lived voice tickets and owns lease heartbeat. Join starts muted. Flags close capture before HTTP authorization; deafen disables local remote tracks and SFU subscription preferences. Failure disconnects and releases the lease, with expiry as crash fallback.

Credentials remain in memory, not preferences/SQLite; restart requires sign-in. Message caches are not encrypted. Linux cache directories are private; Windows inherits user application-data permissions. Snapshots exclude bearer tokens, LiveKit logging is disabled, and logout warns pending sends may already have been delivered.

Existing communities/channels/DMs can be listed and selected; latest 200 cached messages and text send/receive work through the engine. Metadata-only file rows are not downloads. Registration/invite UI, friends, creating DMs/communities, file transfer, editing/reactions, history pagination UI, notifications, mobile, screen sharing, device pickers, updater and persistent secure login remain out of scope. No production infrastructure or PC access was used.

## Automated checks and local evidence

Run flutter analyze and flutter test in apps/porch_flutter. Five injected-bridge widget tests cover sign-in, selection/enqueue, draft retention, channel lists and revocation. The generated native-FFI test intentionally skips without a local fixture.

Build cargo build -p porch-server -p porch-cli -p porch-flutter --locked. With DATABASE_URL set to a LOOPBACK Postgres admin connection, run python3 scripts/flutter-fixture-proof.py. It creates a randomly named database, seeds actual accounts/community/DM/messages through existing CLI contracts, runs generated Dart/native command and stream integration, and drops its own database. Remote DB hosts are rejected. --screenshots additionally needs Xvfb, openbox, xdotool and ImageMagick; coordinates target the exact 1920x1080 Linux layout, not a portable UI automation contract.

The ignored Rust adapter integration runs with cargo test -p porch-flutter --locked -- --ignored and DATABASE_URL. It covers real server HTTP/WS, send acknowledgment, peer gateway receive, secret-free snapshots, logout and account isolation. Hosted Rust CI runs all ignored integration tests. Hosted Flutter Linux and Windows CI builds runnable native bundles; Linux also runs the generated native-FFI fixture.

Evidence docs/evidence/flutter-{signin,home,group,dm}.png is the REAL Linux app using an explicitly LOCAL disposable server fixture, not production or fake widget output. Sam, maya and Friends are seeded test data only, never application defaults. Screenshots do not prove physical audio.

## Exact two-person microphone acceptance

Use two machines/headsets with two DIFFERENT accounts on the same server. Both must already be members of the same community with access to a voice channel. This client signs in but does not register or redeem invites. An administrator uses existing porch-cli invite-create and register commands for instance registration, then community-invite-create and community-join for community membership. PORCH_INVITE, PORCH_USERNAME, PORCH_PASSWORD and PORCH_COMMUNITY_INVITE provide those values. Instance and community invites are separate; create a voice channel through community-mutate if needed. See CLI help/protocol contracts for arguments. Never paste tokens/passwords into issues.

Extract the ENTIRE OS artifact on both machines, launch Porch, use https://api.porch.chat (or another authorized instance), and sign in to the separate accounts. Select the same community tab and voice room. Each joins muted. Ensure OS microphone permissions are enabled and default input/output devices use the headsets; Windows Privacy & security / Microphone controls capture. This slice uses OS default devices, not a custom picker.

Unmute A and speak with B muted; B must hear A. Mute A and confirm B hears no new A audio. Unmute B and reverse direction. With A unmuted, Deafen A and confirm A neither hears nor transmits. Undeafen A leaves capture muted until explicitly unmuted. Leave A, then rejoin and confirm A starts muted. Disconnect/reconnect one network and confirm safe mute/disconnect rather than silent publishing. Sign out while connected and confirm capture/playback stop. Revoke voice permission server-side; after the next heartbeat the client must leave and refuse publishing. Abruptly close an app and verify server lease expiry within configured timeout.

Exchange channel and existing-DM messages both ways. Send during network loss, reconnect and verify a single confirmed message rather than duplicates. Restart/sign in again and verify history. Check Windows scaling, minimum window layout, IME and keyboard focus manually.

Physical microphones, Windows runtime/DPI/IME, production sign-in, acoustic two-person audio, device/reconnect behavior and actual permission-revocation media shutdown are NOT manually proven by sandbox tests. Control-plane tests are not audio proof. LiveKit must already be configured on the chosen instance.

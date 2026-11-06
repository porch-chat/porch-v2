# Flutter desktop client and shared engine

Accepted 2026-11-06 by Dylan. Flutter replaces GPUI as the long-term client direction, with mobile targets later. The existing GPUI client stays buildable during transition.

The approved warm dark Porch design, bundled Manrope and Flyer chat presentation are implemented in apps/porch_flutter. This is own-code UI, not a fork of a previous Porch client.

A flutter_rust_bridge 2.11.1 adapter crate wraps porch-core. Generated Rust and Dart bindings are committed together. Async commands and one snapshot stream expose protocol-serialized JSON records, preserving wire contracts without a second network client. Authentication, account-bound SQLite, durable send nonces, reconciliation, permissions and WebSocket lifetime remain in Rust. JSON snapshots never include bearer credentials.

The official livekit_client Flutter SDK owns device/media transport only. Rust obtains short-lived media tickets, owns voice leases, checks flags/permissions and heartbeats. Join starts muted. Mute/deafen close local capture before server calls; authorization must succeed before publishing. Deafen additionally disables local remote audio tracks and SFU track preferences. Failures disconnect and release the lease; expiry is the crash fallback.

Native bridge libraries are built from the same workspace target cache, explicitly installed into Linux/Windows Flutter bundles by CMake. Hosted Ubuntu/Windows CI builds, analyzes and tests Flutter and uploads entire runnable folders. GPUI CI remains separate and unchanged.

This slice does not add registration/invite management, friends, new DM creation, uploads/downloads, message editing, notifications, screen sharing, device pickers, updater or mobile skeletons. Existing channels and DMs are usable; account/community setup is through existing CLI contracts. Text is rendered as selectable source, never HTML.

# Native desktop v1

The native binary is `porch-desktop`. It uses porch-core against the real HTTP
and gateway backend, not mock data. No server is deployed by this phase.

## Build and run
Use the pinned Rust toolchain. Linux prerequisites (Debian/Ubuntu):
```sh
sudo apt-get install libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libasound2-dev libfontconfig1-dev libssl-dev libvulkan1 fonts-dejavu-core
cargo build -p porch-desktop --locked
cargo run -p porch-desktop --locked
```
GPUI also requires a system fallback font at startup; Manrope is embedded for
the application theme. Use a working Vulkan driver and X11/Wayland session. On Windows install Rust and
Visual Studio C++ build tools / Windows SDK, then the same Cargo commands.
GitHub-hosted Linux and Windows jobs build downloadable debug artifacts with
the bundled font license. These are unsigned development builds, not installers
or auto-updatable releases. Windows graphical/IME/DPI tests remain required.

Sign in using the server address, username and password. Use HTTPS outside
loopback; the shared core refuses insecure remote origins. Platform invitation
registration is optional from the login screen. Password and token are never
saved. The server address is the only account-related preference persisted.
On Windows local state uses LOCALAPPDATA/Porch; on Linux XDG_DATA_HOME/porch or
~/.local/share/porch. PORCH_DATA_DIR overrides it for isolated testing.

Communities appear in the left rail, text channels by category in the sidebar,
direct messages under DM. For v1, start a DM with comma-separated numeric member
IDs; the authorized backend validates them. Member labels other than yourself
are numeric pending a public profile endpoint. Membership is not live presence.

Enter sends, Shift+Enter adds a line. Messages appear optimistically only after
SQLite accepts the send, and the source clears only after that acknowledgement.
Rejected sends have explicit retry/discard controls. Discard warns it cannot
retract delivery. Reply, own-message edit and a thumbs-up toggle use server
contracts. Raw Markdown is plain text, not rich rendered HTML or remote content.
Editing preserves text on errors; navigation is disabled until edit is canceled
or acknowledged to avoid turning an edit into an accidental new send.
Load earlier pages explicitly; Latest returns to the recent 200-message window.
Community/channel management, search/pin/delete controls and polished profile
discovery are not desktop v1 scope, though their server/CLI APIs exist.

Display settings persist font size (12-22px) and the member-column toggle.
Sign out requires a second click, clears cached history and local sends and
warns about unknown delivery. Revocation/sign-out clears UI state too.
The cache is readable SQLite, not encrypted. Local unsent composer text is
in-memory per conversation and disappears on sign-out/exit; only submitted
outbox entries are durable. No desktop session restore/OS keychain yet.

## Evidence and outstanding acceptance
Session-bridge integration tests run against real disposable Postgres and
loopback HTTP/WS without a GPU. They exercise login, DM, durable enqueue,
acknowledged edit, failed edit, invalid selection and sign-out state clearing.
Workspace tests retain the core's reconnect/offline/revocation fault coverage.
Headless Xvfb/Mesa screenshots, when successful, are layout/startup evidence only.
They do not prove real Windows/Linux driver behavior, clipboard, keyboard
layouts, IME composition, DPI/resizing, accessibility or long-session performance.
Dylan performs those graphical checks; no PC was accessed.

Development smoke may set PORCH_URL + PORCH_TOKEN in the environment and
PORCH_SMOKE_SECONDS (1-120) to exit an isolated launch. Never put tokens in
arguments, settings, screenshots or checked-in scripts. No sample/fake chat
is displayed by the production binary.

Local Xvfb + Openbox + Mesa smoke started the native process, authenticated and
reconciled the real cache, then exited cleanly. A real native mouse click and
keyboard Enter through the composer sent a message which was acknowledged by
the loopback backend and reconciled to SQLite. An inspected screenshot shows
the actual rail/sidebar/conversation/member layout. Xvfb without a window manager
produced black captures; Openbox was required in this sandbox. This is headless
software-rendered evidence, not real hardware/IME/DPI or Windows runtime acceptance.

The reproducible Linux smoke is scripts/desktop-smoke.py. Use an empty disposable
Postgres database and run under a 1440x1000 Xvfb display with Openbox, xdotool and
ImageMagick installed. It creates fixture messages, types a fresh fixture username/password through
the actual native sign-in form, types into the real composer, verifies delivery
through the CLI and exits. Password entry uses stdin, never process argv. PORCH_SMOKE_SCREENSHOT optionally saves a fixture-only screenshot. CI
runs it in a separate empty database with Mesa software Vulkan. This smoke
does not exercise other devices or real drivers.

## Captured evidence

[Native Linux software-rendered login and message-delivery smoke](evidence/phase6-linux-headless.png)
uses generated fixture accounts and a disposable local database. It is not a
production deployment or Windows/Dylan hardware runtime acceptance. CI also
includes its own screenshot inside the Linux debug artifact. Download artifacts
from the specific successful run, not a moving latest-build URL; GitHub sign-in
is required for this private repository and artifacts expire after seven days.

[Minimum 950×640 window, software-rendered](evidence/phase6-minimum-size.png)
also passed native login/send before resizing. Stacked author/timestamp metadata
avoids collisions with message actions at this width. This still does not prove
real hardware, operating-system DPI or IME behavior.

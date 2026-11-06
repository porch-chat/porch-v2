# Private files (Phase 7 automated scope complete)

Enable file routes with an absolute PORCH_STORAGE_ROOT whose directory is private
to the server operator (0700 on Unix). Unset means file routes are disabled.
There is no public bucket, user-controlled path or filename-based object lookup.

POST /uploads accepts CreateUpload. GET /uploads/{id} returns acknowledged
received bytes; POST /uploads/{id}/chunks?offset=N accepts raw bytes, fixed 4 MiB
except the final chunk. POST /uploads/{id}/complete streams a SHA-256 check.
DELETE /uploads/{id} aborts an unlinked upload. Every call requires a current
session, ownership and live conversation permissions. IDs alone grant nothing.
Upload expiry is 24 hours from creation, not refreshed by retries.

SendMessage.attachments is an optional array of up to ten finalized upload IDs.
A message may contain attachments without text. Binding, nonce idempotency and
message creation commit in one transaction. Attachments cannot cross owners or
conversations, and cannot be reused by a different message. Edits preserve the
attachment list; tombstones clear it. Old messages decode with an empty list.

GET /attachments/{id} requires current read permission on a nondeleted linked
message. A single Range header is supported, including suffix ranges. Originals
are always application/octet-stream with attachment disposition, no-store and
nosniff; sniffed MIME is metadata, not permission to execute or inline content.
Revoked users and deleted messages lose access at the next request. A transfer
already authorized and streaming is not forcibly interrupted by later revocation.

All disk IO runs outside the async reactor. Eight concurrent file operations or
streams per server instance bound chunk buffers and download streams. Chunks
have a 15-second receive deadline and 4 MiB body cap. Other upload JSON has a
4 KiB cap. Quotas reserve original sizes in Postgres across instances, 10 GiB
per user, 100 GiB total and 16 unlinked objects per user. These are logical
original-size quotas, NOT disk capacity limits. Chunks are retained after
finalization to permit exact retries following a disk/SQL crash gap. A finalized
object uses approximately 2x its original size; re-finalization can transiently
use 3x. Operators need filesystem capacity monitoring and a separate disk budget.
Linked deleted-message objects still count until explicit retention is designed.
No automatic deletion of message-associated bytes is claimed.

Creation reaps up to 32 expired unlinked objects under the reservation lock.
A failed cleanup fails closed rather than releasing reservation accounting.
No traffic means no expiry reclamation. Rows deliberately have no cascade to
silently orphan disk objects. Restores must pair SQL and storage snapshots.
Crash before SQL acknowledgement leaves a replaceable orphan chunk; status
always returns the committed acknowledgement. Finalize after a publication/SQL
gap rebuilds from retained chunks and revalidates the hash before publication.

Tests exercise real Axum HTTP extractors, isolated Postgres schemas and disk,
including disk-published/SQL-unacknowledged failure injection, duplicate races,
ownership/cross-conversation binding, removal, deletion, ranges and mismatch.
The separate ignored storage test streams exactly 1 GiB without a GiB buffer.
It proves storage mechanics, not a 1 GiB full HTTP/UI round trip.

Phase 7 local implementation now includes backend/CLI/shared-core transfers,
raster thumbnails, optional video posters, guarded server-side embeds and native
attachment actions. Local and hosted headless media evidence and Linux/Windows
CI passed at 620bbca. Native runtime, interactive picker/save dialogs, DPI/IME
and hardware acceptance remain Dylan's, separate from automated checks. No
deployment has occurred. See the verification record below.

CLI transfer proof uses file-create CONVERSATION PATH, file-status ID,
file-resume ID PATH, file-upload CONVERSATION PATH, file-complete ID,
file-abort ID and file-download ATTACHMENT NEW_PATH. The token stays in
PORCH_TOKEN. Upload/resume hashes the source in 64 KiB buffers, then sends at
most 4 MiB at once. The upload ID is printed to stderr before sending chunks.
After interruption, explicit file-resume queries the server's acknowledged
position and verifies local size/hash. It never assumes a timed-out request
failed and never appends blindly. Download refuses existing paths, creates
private files on Unix and removes partial files on errors. CLI downloads check
length but not a separately provided digest; the HTTP integration compares exact
bytes. Shared-core durable upload state is described below; authenticated ranged
download resume is not implemented.

Shared core now persists upload ID, source path and last acknowledged status in
account-bound SQLite, with conversation foreign-key cascade. Sign-out purges
that metadata. begin_upload requires synchronized conversation visibility;
advance_upload re-queries the authoritative offset and sends at most one chunk
per call, or finalizes after all bytes. A failed acknowledgement does not advance
the local marker. A fresh engine resumes from the server, not that stale marker.
Every advance rehashes the source before reading the chunk; this conservative
v1 choice detects edits but costs O(file size) disk reads per chunk. Source
mutation after that check still fails the server's final whole-file digest.
Only sanitized transport errors escape; local paths are never sent to the server.
Concurrent calls may return conflict/stale-ack errors; no automatic append retry
or cross-process local upload scheduling is claimed. queue_attachments supports
attachment-only durable sends. Ready transfer metadata stays until explicit
cancel (for unsent objects) or conversation/account purge. Durable ranged
downloads are not implemented.

## Generated previews and explicit link metadata

GET /attachments/{id}/preview reauthorizes the same live-message scope as a
download, then decodes only PNG/JPEG/WebP under a 16 MiB compressed-input cap,
4096-by-4096 dimension limit and 96 MiB best-effort decode allocation limit.
It re-encodes to a metadata-free PNG at most 480 by 320. Eight bounded blocking
workers prevent unbounded simultaneous decodes. There is no preview cache.
This is not a process sandbox or a hard CPU-time/memory guarantee against a
decoder vulnerability. HTML/SVG/scripts and videos do not use the raster decoder.

When explicitly enabled with PORCH_EMBEDS=1, POST
/conversations/{conversation}/messages/{message}/embed accepts {"url":"..."}.
The exact URL must occur as a Markdown link in a live readable message.
It returns plain bounded title/description strings, never remote HTML or image
URLs. HTTPS port 443 only, no credentials/fragments, all resolved addresses must
be public IPv4 and are pinned for the request. IPv6, mapped/transition forms,
IANA special-purpose IPv4 ranges, mixed public/private DNS, redirects, ambient
proxies and compressed responses are denied. No cookies or authorization are
sent. Total DNS/network deadline is five seconds, request timeout four seconds,
HTML body at most 1 MiB. Four workers, six requests/user/minute and sixty global
requests/minute; at most 1024 rate-map keys. Limits are per process, not cluster
quotas. Metadata parsing stays on the bounded blocking pool; current permission,
message liveness and session authentication are checked again after fetching.
The IANA address policy is conservative and must be maintained when registries
change. Outbound fetching is disabled by default, including if storage is enabled.
No remote success integration or production network egress is claimed by the
hermetic policy/response/authorization tests.

Native media actions
--------------------

The desktop stages one file through the platform file picker, hashes it on the
session worker, and does not transmit contents until Upload/Resume is clicked.
One authoritative-offset chunk advances per worker tick; progress uses committed
bytes, and reconnect failures stop until explicit Resume. Cancel is processed
after any in-flight chunk/request, not an instantaneous socket interruption.
Ready files have a separate Send file action, backed by the durable outbox.
The source path stays only in the account-bound SQLite cache.

Received cards save through a native destination dialog with create-new,
private permissions, streamed bounded bytes and SHA-256 verification. Existing
destinations are never overwritten; failures/cancellation remove partial files.
Downloads currently use the common 20-second request timeout and do not resume.
An original is never launched or handed to a shell. Save requires the referenced
message in the most recent 200 cached messages for that conversation.

Load preview explicitly requests an authenticated server PNG, verifies bounded
PNG dimensions/decoding off the render thread, and keeps at most eight previews
in memory. Navigation, message deletion and account changes clear visible
previews. Link details are an explicit opt-in per Markdown link, rendered as
plain text with no remote images. The picker/save dialogs use GPUI platform APIs,
not custom filesystem browsing. Linux/Windows interactive dialog, DPI, IME and
assistive-technology acceptance still requires real user testing.

Optional video posters
----------------------

Linux servers can opt in with absolute PORCH_FFMPEG_BIN and PORCH_PRLIMIT_BIN
paths to maintained FFmpeg and util-linux prlimit executables. Disabled by
default; missing/relative/non-Linux configurations fail startup. Decoding remains
opt-in per native Load preview click. The server pipes only a private object's
bytes, never its name or a URL, into the child. Only MP4/WebM containers and
H.264/HEVC/VP8/VP9/AV1 decoders are allowed; only the pipe protocol is enabled.
No nested files/network/playlists, audio/subtitles, inherited environment,
stderr body or original video is exposed to the native renderer.

Video originals up to 64 MiB and 8,388,608 pixels may yield one static 480x320
PNG poster. This is not inline playback or an animated thumbnail. Pipe-based
MP4 decoding may fail for non-streamable files; any unsupported/invalid video
remains downloadable, never auto-opened. Two concurrent decoder processes,
five-second wall timeout, four CPU seconds, 512 MiB virtual-address-space and
32-file-descriptor limits apply. Child stack is capped at 8 MiB and glibc
allocator arenas at two (MALLOC_ARENA_MAX=2); OpenMP/OpenBLAS/MKL threads are
capped at one. These reduce reservations without increasing the 512 MiB cap.
FFmpeg allocations are individually capped at
32 MiB and output at 2 MiB before a second raster validation/re-encoding pass.
Cancellation kills the child; process drop also has kill-on-drop enabled.

These limits/protocol allowlists are NOT an OS sandbox against decoder exploits.
Before enabling for untrusted users, run the service/decoder with a low-privilege
identity in a separately restricted container or OS sandbox with no egress or
access to secrets, keep FFmpeg patched, and budget storage/CPU separately.
No such deployment or isolation configuration has been performed in this phase.
All generated media is fixture-only; tests exercise real FFmpeg poster output,
default-disabled behavior, auth/scope/deletion denial, excessive size/concurrency,
and malformed HTML, playlist, local-file-reference and truncated container input.


## Verification record, 2026-11-06

Phase 7 automated scope passed locally and on GitHub-hosted Linux and Windows
at 620bbca, run https://github.com/porch-chat/porch-v2/actions/runs/42193393398
Linux passed fmt/clippy, 49 default and all 12 explicitly ignored tests, doctests,
workspace build, real CLI smoke, interactive headless login/composer and
authenticated native media poster smoke. Windows passed desktop clippy, tests
and build; this is not Windows runtime acceptance.

Linux artifact (includes native screenshots):
https://github.com/porch-chat/porch-v2/actions/runs/42193393398/artifacts/13491626131
Windows artifact:
https://github.com/porch-chat/porch-v2/actions/runs/42193393398/artifacts/13490404786
Artifacts are private debug builds with seven-day retention, not releases.

Local native pixel proof is docs/evidence/phase7-linux-media.png, produced by
scripts/desktop-media-smoke.py using an actual authenticated upload and explicit
Load preview click. Two interior pixels assert the generated amber poster.
Hosted Linux repeats that harness, not just an image-only unit test.
The 1 GiB test proves streaming storage, not a full 1 GiB HTTP/UI transfer.
Remote embed success, video playback, interactive picker/save dialogs, Windows
runtime and real hardware/DPI/IME acceptance are not claimed. FFmpeg remains
disabled by default and opt-in limits are not an OS sandbox. Phase 8 deployment
is not authorized and no deployment or infrastructure changes occurred.

# Private files (Phase 7 in progress)

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

Still required before Phase 7 completion: CLI/shared-core transfer client with
durable resume, bounded generated image thumbnails, actual native file UI and
image/video previews, guarded server-side link embeds, end-to-end client tests,
headless native UI proof, and final Linux/Windows hosted CI. Native runtime,
DPI/IME/hardware acceptance remains Dylan's. No deployment has occurred.

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
bytes. Shared-core durable transfer state and authenticated ranged download
resume are still pending, not implied by these terminal commands.

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
cancel (for unsent objects) or conversation/account purge; a native cleanup UX
and durable ranged downloads are still pending.

Image previews are now available at authenticated GET /attachments/{id}/preview.
Only attached, live, readable messages qualify, with identical scope checks to
original downloads. PNG/JPEG/WebP inputs up to 16 MiB and strict 4096x4096 pixel
bounds are decoded in bounded blocking work and re-encoded as metadata-free PNG
at most 480x320. SVG/HTML and other codecs are never rendered. Decoder allocation
limit 96 MiB is best effort, not an OS memory/CPU sandbox or hard time limit.
There is no persistent preview cache yet; requesting again repeats bounded work.
Actual HTTP tests cover authorized raster output, wrong scope, unsent objects
and deletion denial. Video previews and native display remain pending.

## Generated previews and explicit link metadata (phase still incomplete)

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

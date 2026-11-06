# ADR-0008: scoped resumable objects and untrusted previews
Status: in progress, Phase 7 is NOT complete
Date: 2026-11-06

Contract first. Files are private scoped objects, not public URLs or filesystem
paths. Upload IDs are 128-bit random lowercase hex; filenames are display-only.
1 GiB original limit, sequential fixed 4 MiB chunks, final short chunk allowed.
A client resumes from the acknowledged offset, never an assumed network write.
Identical retries return current status; differing bytes/offsets conflict.

Storage is a synchronous trait executed only on bounded blocking workers. Local
disk uses an operator-owned private root, private temporary files and atomic
rename; SQL owns acknowledgement. Chunk files are durable before SQL advances.
After a crash, an unacknowledged orphan chunk is replaced at the same offset.
Finalization streams chunks through SHA-256 into a private temporary original;
only a matching size/hash is published atomically. No 1 GiB Vec or DB blob.
The original is immutable after Ready. Retried finalization is idempotent.
Local directory fsync covers rename durability on Unix.

Every route authenticates again, rechecks current conversation permission and
ownership, and serializes conversation then upload row. Send binds up to ten
Ready objects in the same transaction as nonce/fingerprint and message creation.
A deleted message has no attachment metadata. Downloads require a live message
link plus current VIEW/READ, never possession of an upload ID. Originals always
download as attachment with no-store/nosniff and fixed safe response headers.
Range is single bounded byte-range only; authenticated readers can stream video.
File type signatures are NOT a safety guarantee; nothing executes uploaded bytes.

Quota creation uses a global advisory transaction lock, counting complete and
incomplete reserved bytes, 10 GiB/user and 100 GiB/global plus 16 pending/user.
24-hour unlinked uploads expire, cannot be mutated/attached, and are reaped in
bounded batches during creation. Linked originals are retained until an explicit
retention design. Object rows intentionally have no cascading foreign keys that
would silently orphan disk objects. Restore must keep database and storage paired.
Private root must not be writable by another principal; same-UID/root compromise
is outside this boundary, not magically solved by validating object keys.

Use maintained tempfile (1472 GitHub stars) and infer (414), checked 2026-11-06;
signature detection only, no arbitrary decode. chub lacks Rust entries, so exact
docs.rs and downloaded source are API references. SHA-256 is existing sha2.

Implementation now includes CLI/adversarial HTTP proofs, durable shared-core
resume, bounded raster thumbnails, native file actions and guarded opt-in HTTPS
metadata. Optional Linux video posters use maintained FFmpeg via stdin/stdout,
prlimit and strict protocols/codecs/formats. These are defense-in-depth limits,
not a decoder exploit sandbox; disabled by default, require operator isolation
before use with untrusted users. See files.md for exact limits and configuration.
Static posters are previews, not video playback. No deployment or PC access.

Final headless native media evidence and hosted Linux/Windows CI are outstanding
Phase 7 gates; real platform dialogs/hardware/DPI/IME acceptance remains Dylan's.

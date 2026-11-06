# ADR-0009: Porch voice leases and replaceable media transport

Status: implementation in progress, Phase 8 separately authorized 2026-11-06.

Porch owns voice authorization, channel addressing, participant state and leases.
VoiceProvider is the server seam; LiveKit is its first adapter, not a public
permission authority. Native clients use a separate media adapter. No old Porch
code. The existing warm pine/amber sidebar gains a compact voice control shelf;
voice channels remain distinct from text conversations.

Join requires current VIEW_CHANNEL + CONNECT on a Voice channel. SPEAK separately
controls microphone-only publish permission; data/video/metadata writes are denied.
Tokens expire after 60 seconds, are returned no-store, never Debug/logged/cached.
One active lease per user, tied to the authenticating session and a random identity.
State changes require that lease and session. Deafen implies effective mute.
Speaking is ephemeral client telemetry, not a server trust/security signal.
Gateway VoiceChanged is a payload-free invalidation; snapshots always reauthorize,
so replay never leaks channel IDs or revoked participant information.

A five-second sweeper checks session validity, live permissions and a 30-second
HTTP lease heartbeat. Lost CONNECT or changed SPEAK disconnects the participant;
a new join is required. SFU RPC failure retains cleanup work for retry rather than
silently forgetting a live participant. Restart evicts participants from only this dedicated
instance's Porch-prefixed rooms before admitting voice. Short token lifetime is
not itself media revocation. No cross-node/outbox guarantees. Real hardware voice
acceptance remains Dylan's; compile and synthetic media are distinct evidence.

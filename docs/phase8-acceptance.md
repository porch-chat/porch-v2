# Phase 8 hardware acceptance

This checklist is not recorded as passed by compiling or using fake media.
Dylan owns graphical/microphone testing; Kat does not access his PC.

Download matching Linux and Windows desktop artifacts from the final green CI run.
Install Linux runtime audio/GLib libraries if needed; allow Windows microphone
privacy permission. Use two different Porch users and actual headsets to avoid
feedback. Sign in at the approved public API origin once route migration is agreed;
loopback deployment alone is not a public acceptance environment.

Join Front steps on both clients. Both must start microphone-muted. Unmute, speak
in both directions, and confirm the speaking marker belongs to the correct user.
Mute must stop remote sound immediately, even during API latency. Deafen must mute
your mic and all remote audio; listening again must not silently unmute your mic.
Cycle Mic/Output device buttons across real devices and confirm correct routing.
Enable push-to-talk, explicitly unmute, then hold/release F9 while Porch is focused.
Release and switching to another window must immediately silence the mic. This is
focused-window PTT, NOT a system-wide hotkey. Test keyboard repeat and queue pressure.
Leave and close the app; remote participant should disappear. Repeat logout, network
loss/reconnect, and rejoin; never auto-open a microphone. Verify permission revocation
(CONNECT and SPEAK separately), session revocation, listen-only member, and a member
who cannot view/connect. Direct HTTP denials do not alone prove SFU revocation timing.

Try two separate external networks. Current transport offers direct UDP 39782 with
TCP 39781 fallback, no TURN. Report restrictive-network failures without claiming
universal connectivity. Capture OS/build, device names and direction when reporting.

Headless native evidence is saved at `docs/evidence/phase8-voice.png`.
This is a real GPUI client logged into the deployed API and joined to LiveKit,
with mute/deafen/leave, push-to-talk, participant state and device controls visible.
The participant indicator is idle/muted, not proof of detected speech.
Audio devices are synthetic PulseAudio null/monitor devices. This screenshot
does not establish microphone capture, audible playback or two-person media.

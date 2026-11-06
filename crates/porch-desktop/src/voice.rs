//! Native media adapter. Porch protocol remains the only control-plane contract.
use livekit::{PlatformAudio, options::TrackPublishOptions, prelude::*};
use porch_core::{Api, Error, Result};
use porch_protocol::*;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Clone, Default)]
pub struct Gate(Arc<Mutex<(bool, Option<LocalAudioTrack>)>>);
impl Gate {
    pub fn allow(&self, allowed: bool) {
        let mut g = self.0.lock().unwrap();
        g.0 = allowed;
        if !allowed && let Some(m) = &g.1 {
            m.disable();
            m.mute();
        }
    }
    fn attach(&self, mic: Option<LocalAudioTrack>) {
        let mut g = self.0.lock().unwrap();
        g.0 = false;
        g.1 = mic;
    }
    fn open(&self) {
        let g = self.0.lock().unwrap();
        if let Some(m) = &g.1 {
            if g.0 {
                m.enable();
                m.unmute();
            } else {
                m.disable();
                m.mute();
            }
        }
    }
}

use tokio::sync::mpsc;

#[derive(Clone, Default)]
pub struct View {
    pub channel: Option<VoiceChannel>,
    pub flags: VoiceFlags,
    pub ptt: bool,
    pub manual_muted: bool,
    pub participants: Vec<VoiceParticipant>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub input: usize,
    pub output: usize,
}
pub struct Voice {
    gate: Gate,
    api: Api,
    ticket: VoiceTicket,
    room: Room,
    audio: PlatformAudio,
    mic: Option<LocalAudioTrack>,
    events: mpsc::UnboundedReceiver<RoomEvent>,
    view: View,
    last: Instant,
}
impl Voice {
    pub async fn join(api: Api, channel: VoiceChannel, gate: Gate) -> Result<Self> {
        let ticket: VoiceTicket = api.post("/voice/join", &VoiceJoin { channel }).await?;
        match Self::connect(api.clone(), ticket.clone(), gate).await {
            Ok(v) => Ok(v),
            Err(e) => {
                let _: Result<bool> = api
                    .post(
                        "/voice/leave",
                        &VoiceLeave {
                            lease: ticket.lease,
                        },
                    )
                    .await;
                Err(e)
            }
        }
    }
    async fn connect(api: Api, ticket: VoiceTicket, gate: Gate) -> Result<Self> {
        let audio = PlatformAudio::new().map_err(|_| Error::Transport)?;
        let inputs = audio.recording_devices().map(|d| d.name).collect();
        let outputs = audio.playout_devices().map(|d| d.name).collect();
        let VoiceTransport::Livekit { url, token } = &ticket.transport;
        let (room, events) = tokio::time::timeout(
            Duration::from_secs(15),
            Room::connect(url, token, RoomOptions::default()),
        )
        .await
        .map_err(|_| Error::Transport)?
        .map_err(|_| Error::Transport)?;
        let mic = if ticket.can_speak {
            let mic = LocalAudioTrack::create_audio_track("Porch microphone", audio.rtc_source());
            mic.disable();
            mic.mute();
            if room
                .local_participant()
                .publish_track(
                    LocalTrack::Audio(mic.clone()),
                    TrackPublishOptions {
                        source: TrackSource::Microphone,
                        ..Default::default()
                    },
                )
                .await
                .is_err()
            {
                let _ = room.close().await;
                return Err(Error::Transport);
            }
            Some(mic)
        } else {
            None
        };
        gate.attach(mic.clone());
        Ok(Self {
            gate,
            api,
            ticket: ticket.clone(),
            room,
            audio,
            mic,
            events,
            view: View {
                channel: Some(ticket.channel),
                flags: VoiceFlags {
                    muted: true,
                    ..Default::default()
                },
                manual_muted: true,
                inputs,
                outputs,
                ..Default::default()
            },
            last: Instant::now() - Duration::from_secs(10),
        })
    }
    pub fn view(&self) -> View {
        self.view.clone()
    }
    pub async fn leave(&mut self) {
        self.silence();
        let _ = self.room.close().await;
        let _: Result<bool> = self
            .api
            .post(
                "/voice/leave",
                &VoiceLeave {
                    lease: self.ticket.lease.clone(),
                },
            )
            .await;
    }
    fn silence(&self) {
        self.gate.allow(false);
        if let Some(m) = &self.mic {
            m.disable();
            m.mute();
        }
        let _ = self.audio.stop_recording();
    }
    pub async fn flags(
        &mut self,
        muted: bool,
        deafened: bool,
        ptt: bool,
        held: bool,
    ) -> Result<()> {
        self.view.manual_muted = muted;
        self.view.ptt = ptt;
        let flags = VoiceFlags {
            muted: muted || (ptt && !held),
            deafened,
            speaking: false,
        }
        .effective(self.ticket.can_speak);
        // Apply locally first. HTTP failure must never leave the microphone publishing.
        if flags.muted {
            self.silence();
        } else if self.mic.is_some() {
            self.audio.start_recording().map_err(|_| Error::Transport)?;
            self.gate.open();
        }
        for participant in self.room.remote_participants().values() {
            for p in participant.track_publications().values() {
                if let Some(RemoteTrack::Audio(t)) = p.track() {
                    if deafened { t.disable() } else { t.enable() }
                }
            }
        }
        self.view.flags = flags;
        let result = self
            .api
            .post::<_, VoiceFlags>(
                "/voice/state",
                &VoiceUpdate {
                    lease: self.ticket.lease.clone(),
                    flags,
                },
            )
            .await;
        if result.is_err() {
            self.silence();
        }
        result?;
        self.last = Instant::now();
        Ok(())
    }
    pub fn device(&mut self, input: bool, index: usize) -> Result<()> {
        if input {
            self.audio
                .switch_recording_device_by_index(index)
                .map_err(|_| Error::Transport)?;
            self.view.input = index;
        } else {
            self.audio
                .switch_playout_device_by_index(index)
                .map_err(|_| Error::Transport)?;
            self.view.output = index;
        }
        Ok(())
    }
    pub async fn tick(&mut self) -> Result<()> {
        while let Ok(event) = self.events.try_recv() {
            match event {
                RoomEvent::Disconnected { .. } => {
                    self.silence();
                    return Err(Error::Transport);
                }
                RoomEvent::TrackSubscribed {
                    track: RemoteTrack::Audio(t),
                    ..
                } => {
                    if self.view.flags.deafened {
                        t.disable();
                    }
                }
                RoomEvent::ActiveSpeakersChanged { speakers } => {
                    self.view.flags.speaking = !self.view.flags.muted
                        && speakers
                            .iter()
                            .any(|p| p.identity() == self.room.local_participant().identity());
                }
                _ => {}
            }
        }
        if self.last.elapsed() >= Duration::from_millis(self.ticket.heartbeat_ms) {
            let _: VoiceFlags = self
                .api
                .post(
                    "/voice/state",
                    &VoiceUpdate {
                        lease: self.ticket.lease.clone(),
                        flags: self.view.flags,
                    },
                )
                .await?;
            self.last = Instant::now();
        }
        let c = self.ticket.channel;
        let snapshot: VoiceSnapshot = self
            .api
            .get(&format!("/voice/{}/{}", c.community_id, c.channel_id))
            .await?;
        self.view.participants = snapshot.participants;
        Ok(())
    }
}
impl Drop for Voice {
    fn drop(&mut self) {
        self.silence();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gate_defaults_closed_and_revokes_without_worker() {
        let gate = Gate::default();
        assert!(!gate.0.lock().unwrap().0);
        gate.allow(true);
        assert!(gate.0.lock().unwrap().0);
        let ui = gate.clone();
        ui.allow(false);
        assert!(!gate.0.lock().unwrap().0);
        gate.attach(None);
        assert!(!gate.0.lock().unwrap().0);
    }
    #[test]
    fn push_to_talk_and_deafen_never_bypass_server_speak() {
        for muted in [false, true] {
            for held in [false, true] {
                for deafened in [false, true] {
                    let flags = VoiceFlags {
                        muted: muted || !held,
                        deafened,
                        speaking: true,
                    }
                    .effective(false);
                    assert!(flags.muted);
                    assert!(!flags.speaking);
                }
            }
        }
    }
}

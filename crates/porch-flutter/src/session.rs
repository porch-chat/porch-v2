use porch_core::{Engine, Error, GatewayTask};
use porch_protocol::*;
use serde_json::json;
use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
#[derive(Default)]
pub(crate) struct Session {
    pub engine: Option<Engine>,
    pub gateway: Option<GatewayTask>,
    pub selected: Option<i64>,
    pub voice: Option<VoiceTicket>,
    pub flags: VoiceFlags,
    pub heartbeat: Option<Instant>,
    pub voice_error: bool,
}
pub(crate) static SESSION: LazyLock<Mutex<Session>> =
    LazyLock::new(|| Mutex::new(Session::default()));
pub(crate) type Result<T> = anyhow::Result<T>;
pub(crate) fn safe(e: Error) -> anyhow::Error {
    anyhow::anyhow!(e.to_string())
}

impl Session {
    pub fn engine(&self) -> Result<Engine> {
        self.engine
            .clone()
            .filter(|e| !e.is_signed_out())
            .ok_or_else(|| anyhow::anyhow!("Sign in required"))
    }
    pub async fn leave(&mut self) {
        if let Some(ticket) = self.voice.take()
            && let Some(engine) = &self.engine
        {
            let _: porch_core::Result<bool> = engine
                .api()
                .post(
                    "/voice/leave",
                    &VoiceLeave {
                        lease: ticket.lease,
                    },
                )
                .await;
        }
        self.flags = VoiceFlags::default();
        self.heartbeat = None;
    }
    pub async fn close(&mut self) {
        self.leave().await;
        if let Some(gateway) = self.gateway.take() {
            gateway.stop().await;
        }
        if let Some(engine) = self.engine.take() {
            let _ = engine.logout().await;
        }
        self.selected = None;
    }
    pub async fn snapshot(&mut self) -> Result<String> {
        let Some(engine) = self.engine.clone() else {
            return Ok(json!({"user":null}).to_string());
        };
        if engine.is_signed_out() {
            self.close().await;
            return Ok(json!({"user":null,"notice":"Session ended. Removed local sends may already have been delivered."}).to_string());
        }
        if let Some(ticket) = &self.voice
            && self.heartbeat.is_none_or(|last| {
                last.elapsed() >= Duration::from_millis(ticket.heartbeat_ms.min(10000))
            })
        {
            let result: porch_core::Result<VoiceFlags> = engine
                .api()
                .post(
                    "/voice/state",
                    &VoiceUpdate {
                        lease: ticket.lease.clone(),
                        flags: self.flags,
                    },
                )
                .await;
            match result {
                Ok(flags) => {
                    self.flags = flags;
                    self.heartbeat = Some(Instant::now());
                }
                Err(_) => {
                    self.leave().await;
                    self.voice_error = true;
                }
            }
        }
        let cache = engine.cache();
        let heads = cache.heads().await.map_err(safe)?;
        if !heads.iter().any(|h| Some(h.id) == self.selected) {
            self.selected = heads.first().map(|h| h.id);
        }
        let messages = if let Some(id) = self.selected {
            cache.messages(id, None, 200).await.map_err(safe)?
        } else {
            vec![]
        };
        let pending = cache.pending().await.map_err(safe)?;
        let notice = cache.removal_notice().await.map_err(safe)?;
        Ok(json!({
            "user":engine.user(),"communities":cache.communities().await.map_err(safe)?,
            "heads":heads,"selected":self.selected,"messages":messages,"pending":pending,
            "notice":notice.map(|_| "Removed local sends may already have been delivered."),
            "connection": self.gateway.as_ref().map(|g| format!("{:?}", *g.state().borrow())).unwrap_or("Stopped".into()),
            "voice":self.voice.as_ref().map(|t| json!({"channel":t.channel,"flags":self.flags,"can_speak":t.can_speak})),
            "presence":engine.presence(),
            "voice_error":self.voice_error
        }).to_string())
    }
}

use crate::frb_generated::StreamSink;
use crate::session::{Result, SESSION, safe};
use porch_core::{Api, Engine};
use porch_protocol::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

/// Credentials are memory-only. Cache name binds origin and server-authenticated user.
pub async fn login(
    origin: String,
    username: String,
    password: String,
    cache_root: String,
) -> Result<()> {
    let mut session = SESSION.lock().await;
    session.close().await;
    let api = Api::new(&origin, "login".into()).map_err(safe)?;
    let auth: AuthResponse = api
        .post("/auth/login", &LoginRequest { username, password })
        .await
        .map_err(safe)?;
    let root = PathBuf::from(cache_root);
    std::fs::create_dir_all(&root).map_err(|_| anyhow::anyhow!("Cache directory unavailable"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| anyhow::anyhow!("Cache permissions"))?;
    }
    let key = format!(
        "{:x}",
        Sha256::digest(format!("{}:{}", api.origin(), auth.user.id))
    );
    let engine = Engine::open(api.origin(), auth.token, root.join(format!("{key}.sqlite")))
        .await
        .map_err(safe)?;
    engine.reconcile().await.map_err(safe)?;
    session.gateway = Some(engine.start_gateway().map_err(safe)?);
    session.engine = Some(engine);
    session.voice_error = false;
    Ok(())
}
pub async fn logout() {
    SESSION.lock().await.close().await;
}
pub async fn select_conversation(id: i64) -> Result<()> {
    let mut s = SESSION.lock().await;
    if !s
        .engine()?
        .cache()
        .heads()
        .await
        .map_err(safe)?
        .iter()
        .any(|h| h.id == id)
    {
        return Err(anyhow::anyhow!("Conversation unavailable"));
    }
    s.selected = Some(id);
    Ok(())
}
/// Returns after durable enqueue, not network acknowledgment. No blind retry on timeout.
pub async fn send_message(conversation: i64, text: String) -> Result<()> {
    let engine = SESSION.lock().await.engine()?;
    engine
        .queue(
            conversation,
            MessageContent::Markdown { text },
            None,
            vec![],
            false,
        )
        .await
        .map_err(safe)?;
    tokio::spawn(async move {
        let _ = engine.flush().await;
    });
    Ok(())
}
pub async fn snapshot() -> Result<String> {
    SESSION.lock().await.snapshot().await
}
/// One subscriber per app. Cancellation closes the session and releases voice leases.
pub async fn snapshots(sink: StreamSink<String>) {
    let mut timer = tokio::time::interval(Duration::from_millis(500));
    loop {
        timer.tick().await;
        let result = SESSION.lock().await.snapshot().await;
        let value = result.unwrap_or_else(|_| json!({"error":"Cache unavailable"}).to_string());
        if sink.add(value).is_err() {
            SESSION.lock().await.close().await;
            break;
        }
    }
}
/// The only secret-bearing bridge response, handed directly to the official media SDK.
pub async fn join_voice(community: i64, channel: i64) -> Result<String> {
    let mut s = SESSION.lock().await;
    s.leave().await;
    let ticket: VoiceTicket = s
        .engine()?
        .api()
        .post(
            "/voice/join",
            &VoiceJoin {
                channel: VoiceChannel {
                    community_id: community,
                    channel_id: channel,
                },
            },
        )
        .await
        .map_err(safe)?;
    let output =
        serde_json::to_string(&ticket).map_err(|_| anyhow::anyhow!("Invalid voice ticket"))?;
    s.flags = VoiceFlags {
        muted: true,
        ..Default::default()
    };
    s.heartbeat = None;
    s.voice_error = false;
    s.voice = Some(ticket);
    Ok(output)
}
pub async fn leave_voice() {
    SESSION.lock().await.leave().await;
}
pub async fn voice_flags(muted: bool, deafened: bool) -> Result<()> {
    let mut s = SESSION.lock().await;
    let t = s
        .voice
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not connected"))?;
    let flags = VoiceFlags {
        muted,
        deafened,
        speaking: false,
    }
    .effective(t.can_speak);
    let result: porch_core::Result<VoiceFlags> = s
        .engine()?
        .api()
        .post(
            "/voice/state",
            &VoiceUpdate {
                lease: t.lease.clone(),
                flags,
            },
        )
        .await;
    match result {
        Ok(flags) => {
            s.flags = flags;
            s.heartbeat = Some(Instant::now());
            Ok(())
        }
        Err(e) => {
            s.leave().await;
            Err(safe(e))
        }
    }
}

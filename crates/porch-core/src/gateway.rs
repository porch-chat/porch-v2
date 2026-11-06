use crate::{Engine, Error, Result};
use futures_util::{SinkExt, StreamExt};
use porch_protocol::*;
use rand::Rng;
use std::time::{Duration, Instant};
use tokio::{sync::watch, task::JoinHandle};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Online,
    Offline,
    Reconnecting,
    Unauthorized,
    Stopped,
}
/// Owns exactly one WebSocket task and its coalesced sync worker. Drop cancels both.
pub struct GatewayTask {
    stop: watch::Sender<bool>,
    state: watch::Receiver<ConnectionState>,
    handle: Option<JoinHandle<()>>,
}
impl GatewayTask {
    pub fn state(&self) -> watch::Receiver<ConnectionState> {
        self.state.clone()
    }
    pub async fn stop(mut self) {
        let _ = self.stop.send(true);
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}
impl Drop for GatewayTask {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}
impl Engine {
    pub fn start_gateway(&self) -> Result<GatewayTask> {
        if self
            .inner
            .gateway_active
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(Error::InvalidData);
        }
        let (stop, stop_rx) = watch::channel(false);
        let (state, state_rx) = watch::channel(ConnectionState::Connecting);
        let engine = self.clone();
        let handle = tokio::spawn(async move {
            let (wake, wakes) = watch::channel(0u64);
            let guard = engine.clone();
            let worker = sync_worker(engine.clone(), stop_rx.clone(), wakes);
            let socket = socket_worker(engine, stop_rx, state.clone(), wake);
            tokio::join!(worker, socket);
            guard
                .inner
                .gateway_active
                .store(false, std::sync::atomic::Ordering::SeqCst);
            if *state.borrow() != ConnectionState::Unauthorized {
                let _ = state.send(ConnectionState::Stopped);
            }
        });
        Ok(GatewayTask {
            stop,
            state: state_rx,
            handle: Some(handle),
        })
    }
}
async fn sync_worker(
    engine: Engine,
    mut stop: watch::Receiver<bool>,
    mut wakes: watch::Receiver<u64>,
) {
    let mut periodic = tokio::time::interval(Duration::from_secs(15));
    loop {
        tokio::select! {biased;
            _=stop.changed()=>break,
            _=periodic.tick()=>{},
            change=wakes.changed()=>{if change.is_err(){break}}
        }
        if *stop.borrow() || engine.is_signed_out() {
            break;
        }
        // Heartbeats run independently, even during large delta reconciliation.
        tokio::select! {biased;
            _=stop.changed()=>break,
            _=engine.flush()=>{}
        }
    }
}
fn backoff(attempt: u32) -> Duration {
    let cap = (500u64.saturating_mul(1u64 << attempt.min(6))).min(30000);
    Duration::from_millis(rand::thread_rng().gen_range(cap / 2..=cap))
}
async fn socket_worker(
    engine: Engine,
    mut stop: watch::Receiver<bool>,
    state: watch::Sender<ConnectionState>,
    wake: watch::Sender<u64>,
) {
    let mut cursor = None;
    let mut attempt = 0;
    loop {
        if *stop.borrow() || engine.is_signed_out() {
            if engine.is_signed_out() {
                let _ = state.send(ConnectionState::Unauthorized);
            }
            break;
        }
        let _ = state.send(if attempt == 0 {
            ConnectionState::Connecting
        } else {
            ConnectionState::Reconnecting
        });
        let result = tokio::select! {biased;
            _=stop.changed()=>break,
            result=session(&engine,&mut cursor,&state,&wake)=>result
        };
        if result.as_ref().err().is_some_and(Error::unauthorized) {
            let _ = engine.expire().await;
            let _ = state.send(ConnectionState::Unauthorized);
            break;
        }
        if matches!(result, Err(Error::InvalidData)) {
            cursor = None;
        }
        let _ = state.send(ConnectionState::Offline);
        if result.is_ok() {
            attempt = 0
        } else {
            attempt = (attempt + 1).min(10)
        }
        tokio::select! {_ =stop.changed()=>break,_=tokio::time::sleep(backoff(attempt))=>{}}
    }
}
async fn session(
    engine: &Engine,
    cursor: &mut Option<ResumeCursor>,
    state: &watch::Sender<ConnectionState>,
    wake: &watch::Sender<u64>,
) -> Result<()> {
    let mut request = format!(
        "{}/gateway",
        engine.inner.api.origin().replacen("http", "ws", 1)
    )
    .into_client_request()
    .map_err(|_| Error::InvalidData)?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", engine.inner.api.token())
            .parse()
            .map_err(|_| Error::InvalidData)?,
    );
    let connected = tokio::time::timeout(
        Duration::from_secs(20),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map_err(|_| Error::Transport)?;
    let (mut socket, _) = connected.map_err(|e| match e {
        tokio_tungstenite::tungstenite::Error::Http(r) => Error::Http(r.status().as_u16(), None),
        _ => Error::Transport,
    })?;
    send_frame(
        &mut socket,
        Message::Text(
            serde_json::to_string(&GatewayClient::Identify {
                version: PROTOCOL_VERSION,
                resume: cursor.clone(),
            })?
            .into(),
        ),
    )
    .await?;
    let mut interval = tokio::time::interval(Duration::from_secs(15));
    let mut timeout = Duration::from_secs(45);
    let mut last_ack = Instant::now();
    let mut nonce = 0u64;
    let mut ready = false;
    let deadline = tokio::time::sleep(Duration::from_secs(5));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _=&mut deadline, if !ready => return Err(Error::Transport),
            _=interval.tick()=>{
                if engine.is_signed_out(){return Err(Error::SignedOut)}
                if last_ack.elapsed()>timeout{return Err(Error::Transport)}
                nonce=nonce.wrapping_add(1);
                send_frame(&mut socket,Message::Text(serde_json::to_string(&GatewayClient::Heartbeat{nonce})?.into())).await?;
            },
            incoming=socket.next()=>match incoming{
                Some(Ok(Message::Text(text)))=>{
                    if text.len()>65536{return Err(Error::InvalidData)}
                    let message:GatewayServer=serde_json::from_str(&text)?;
                    match message{
                        GatewayServer::Hello{version,heartbeat_interval_ms,heartbeat_timeout_ms}=>{
                            if version!=PROTOCOL_VERSION||!(100..=60000).contains(&heartbeat_interval_ms)||heartbeat_timeout_ms<heartbeat_interval_ms||heartbeat_timeout_ms>180000{return Err(Error::InvalidData)}
                            interval=tokio::time::interval(Duration::from_millis(heartbeat_interval_ms));timeout=Duration::from_millis(heartbeat_timeout_ms);
                        },
                        GatewayServer::Ready{cursor:next,resumed,..}=>{
                            if ready{return Err(Error::InvalidData)}
                            if resumed&&!cursor.as_ref().is_some_and(|c|c.stream==next.stream&&c.sequence<=next.sequence){return Err(Error::InvalidData)}
                            *cursor=Some(next);ready=true;attempt_wake(wake);let _=state.send(ConnectionState::Online);
                        },
                        GatewayServer::Dispatch{envelope}=>{
                            let c=cursor.as_mut().ok_or(Error::InvalidData)?;
                            if !ready||envelope.version!=PROTOCOL_VERSION||envelope.sequence<=c.sequence{return Err(Error::InvalidData)}
                            c.sequence=envelope.sequence;
                            if matches!(envelope.event,GatewayEvent::SyncRequired){attempt_wake(wake)}
                        },
                        GatewayServer::HeartbeatAck{nonce:ack}=>{if ack==nonce{last_ack=Instant::now()}},
                        GatewayServer::Error{code:ErrorCode::Unauthorized|ErrorCode::Forbidden}=>return Err(Error::Http(401,None)),
                        GatewayServer::Error{..}=>return Err(Error::InvalidData),
                    }
                },
                Some(Ok(Message::Ping(p)))=>send_frame(&mut socket,Message::Pong(p)).await?,
                Some(Ok(Message::Close(_)))|None=>return Err(Error::Transport),
                Some(Err(_))=>return Err(Error::Transport),
                _=>{},
            }
        }
    }
}
async fn send_frame(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    message: Message,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), socket.send(message))
        .await
        .map_err(|_| Error::Transport)?
        .map_err(|_| Error::Transport)
}
fn attempt_wake(wake: &watch::Sender<u64>) {
    wake.send_modify(|v| *v = v.wrapping_add(1));
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backoff_is_bounded() {
        for attempt in 0..100 {
            for _ in 0..10 {
                let delay = backoff(attempt);
                assert!((250..=30000).contains(&(delay.as_millis() as u64)))
            }
        }
    }
}

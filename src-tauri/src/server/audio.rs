use super::{close, error, send, send_frame, Context};
use crate::{protocol::ClientMessage, security::Limiter};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::{HeaderMap, StatusCode},
    response::Response,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

// Longer than the sender's 1.2 s outstanding-frame deadline, not a playout budget.
const AUDIO_IDLE_TIMEOUT: Duration = Duration::from_millis(1500);
const AUDIO_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(serde::Deserialize)]
pub(super) struct AudioQuery {
    resume: Option<u64>,
}

pub(super) async fn upgrade_microphone(
    State(ctx): State<Arc<Context>>,
    ConnectInfo(peer): ConnectInfo<crate::transport::Peer>,
    axum::extract::Query(query): axum::extract::Query<AudioQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !ctx.origin_ok(&headers)
        || !ctx
            .limits
            .lock()
            .unwrap()
            .upgrade
            .allow(&peer.0.ip().to_string())
    {
        return error(StatusCode::FORBIDDEN, "来源不匹配或过于频繁");
    }
    let Ok(slot) = ctx.audio_slots.clone().try_acquire_owned() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "音频连接过多");
    };
    peer.websocket_close_grace();
    ws.max_message_size(2048)
        .max_frame_size(2048)
        .write_buffer_size(0)
        .max_write_buffer_size(4096)
        .on_upgrade(move |socket| async move {
            let _slot = slot;
            microphone_connection(socket, ctx, query.resume).await
        })
}
async fn microphone_connection(mut socket: WebSocket, ctx: Arc<Context>, resume: Option<u64>) {
    use crate::microphone::{FrameGuard, RATE, SAMPLES};
    let mut shutdown = ctx.stop_signal();
    let first = tokio::select! { biased; _ = shutdown.changed() => return, message = tokio::time::timeout(Duration::from_secs(3), socket.recv()) => message };
    let token = match first {
        Ok(Some(Ok(Message::Text(text)))) => match ClientMessage::parse(&text) {
            Ok(ClientMessage::Auth { token }) => token,
            _ => {
                close(&mut socket, 4003).await;
                return;
            }
        },
        _ => {
            close(&mut socket, 4003).await;
            return;
        }
    };
    let device = ctx.pairing.lock().unwrap().device(&token);
    let Some(device) = device else {
        close(&mut socket, 4003).await;
        return;
    };
    if !device.permissions.microphone {close(&mut socket,4403).await;return;}
    let audio = ctx.controller.audio.clone();
    let endpoint = {
        let store = ctx.store.lock().unwrap();
        let prefs = store.get();
        audio.set_gain_db(prefs.microphone_gain_db);
        prefs.audio_output
    };
    let id = device.id.clone();
    let name = device.name;
    let Some(prepared) = ctx.while_live(|| audio.prepare_session(&endpoint, &id, &name, resume)) else { return; };
    let (owned_session, ready) = match prepared {
        Ok(value) => value,
        Err(message) => {
            send(
                &mut socket,
                serde_json::json!({"kind":"error","message":message}),
            )
            .await;
            close(&mut socket, 4002).await;
            return;
        }
    };
    let stream = owned_session.stream.clone();
    let session = owned_session.number;
    let mut opening = tokio::task::spawn_blocking(move || ready.wait());
    let mut check = tokio::time::interval(Duration::from_millis(200));
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => return,
            _ = check.tick() => {
                let authorized = ctx.pairing.lock().unwrap().device(&token).is_some_and(|d| d.permissions.microphone);
                if !authorized {
                    stream.stop(); close(&mut socket, 4403).await; return;
                }
            },
            result = &mut opening => {
                let authorized = ctx.pairing.lock().unwrap().device(&token).is_some_and(|d| d.permissions.microphone);
                if !authorized { stream.stop(); close(&mut socket, 4403).await; return; }
                let message = match result {
                    Ok(Ok(())) if !stream.stopped() => break,
                    Ok(Ok(())) => "麦克风启动已取消".into(),
                    Ok(Err(error)) => error,
                    Err(_) => "音频设备启动失败".into(),
                };
                stream.stop();
                send(&mut socket, serde_json::json!({"kind":"error","message":message})).await;
                close(&mut socket, 4002).await;
                return;
            },
            message = socket.recv() => match message {
                Some(Ok(Message::Ping(data))) => {
                    if !send_frame(&mut socket, Message::Pong(data)).await { return; }
                },
                Some(Ok(Message::Pong(_))) => {},
                Some(Ok(Message::Binary(_) | Message::Text(_))) => {
                    stream.stop(); close(&mut socket, 4002).await; return;
                },
                _ => return,
            },
        }
    }
    if !send(
        &mut socket,
        serde_json::json!({"kind":"ready","rate":RATE,"samples":SAMPLES,"window":4,"session":session.to_string()}),
    )
    .await
    {
        return;
    }
    let mut guard = FrameGuard::default();
    let mut last = Instant::now();
    let opened = last;
    let mut last_fresh = None;
    let mut close_code = 4000;
    let mut rate = Limiter::new(150, 1);
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            _ = check.tick() => {
                let authorized = ctx.pairing.lock().unwrap().device(&token).is_some_and(|d| d.permissions.microphone);
                if !authorized {close_code=4403;break;}
                if stream.stopped() {break;}
                let (since, limit) = match last_fresh {
                    Some(fresh) => (fresh, AUDIO_IDLE_TIMEOUT),
                    None => (opened, AUDIO_STARTUP_TIMEOUT),
                };
                if since.elapsed() >= limit { close_code = 4008; break; }
            },
            message = socket.recv() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    if !rate.allow("audio") {break;}
                    let now = Instant::now();
                    if now.duration_since(last)>Duration::from_millis(80) {ctx.controller.audio.note_pressure();}
                    match guard.accept(&bytes,now) {
                        Ok(Some(pcm)) => {
                            last_fresh = Some(now);
                            if stream.push(pcm, u32::from_le_bytes(bytes[8..12].try_into().unwrap()), now) {ctx.controller.audio.note_pressure();}
                        },
                        Ok(None) => ctx.controller.audio.note_pressure(),
                        Err(_) => break,
                    }
                    last = now;
                    let sequence = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
                    let ack = format!(r#"{{"kind":"ack","sequence":{sequence}}}"#);
                    if !send_frame(&mut socket, Message::Text(ack.into())).await {
                        break;
                    }
                },
                Some(Ok(Message::Ping(_)|Message::Pong(_))) => {},
                _ => break,
            }
        }
    }
    stream.stop();
    close(&mut socket, close_code).await;
}

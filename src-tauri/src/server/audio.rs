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
    let token = match tokio::time::timeout(Duration::from_secs(3), socket.recv()).await {
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
    let Some(device) = device.filter(|d| d.permissions.microphone) else {
        close(&mut socket, 4003).await;
        return;
    };
    let endpoint = ctx.store.lock().unwrap().get().audio_output;
    let gain_db = ctx.store.lock().unwrap().get().microphone_gain_db;
    let audio = ctx.controller.audio.clone();
    audio.set_gain_db(gain_db);
    let id = device.id.clone();
    let name = device.name;
    let result =
        tokio::task::spawn_blocking(move || audio.start_session(&endpoint, &id, &name, resume))
            .await;
    let (stream, session) = match result {
        Ok(Ok(stream)) => stream,
        other => {
            let message = match other {
                Ok(Err(e)) => e,
                _ => "音频设备启动失败".into(),
            };
            send(
                &mut socket,
                serde_json::json!({"kind":"error","message":message}),
            )
            .await;
            close(&mut socket, 4002).await;
            return;
        }
    };
    struct Stop(Arc<crate::microphone::Stream>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.stop();
        }
    }
    let _stop = Stop(stream.clone());
    if !send(
        &mut socket,
        serde_json::json!({"kind":"ready","rate":RATE,"samples":SAMPLES,"window":4,"session":session.to_string()}),
    )
    .await
    {
        return;
    }
    let mut guard = FrameGuard::default();
    let mut shutdown = ctx.shutdown.clone();
    let mut check = tokio::time::interval(Duration::from_millis(200));
    let mut last = Instant::now();
    let mut last_fresh = last;
    let mut close_code = 4000;
    let mut rate = Limiter::new(150, 1);
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            _ = check.tick() => {
                let authorized = ctx.pairing.lock().unwrap().device(&token).is_some_and(|d| d.permissions.microphone);
                if !authorized || stream.stopped() {break;}
                if last_fresh.elapsed() >= AUDIO_IDLE_TIMEOUT { close_code = 4008; break; }
            },
            message = socket.recv() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    if !rate.allow("audio") {break;}
                    let now = Instant::now();
                    if now.duration_since(last)>Duration::from_millis(80) {ctx.controller.audio.note_pressure();}
                    match guard.accept(&bytes,now) {
                        Ok(Some(pcm)) => {
                            last_fresh = now;
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

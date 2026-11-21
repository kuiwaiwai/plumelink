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
use futures_util::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub(super) async fn upgrade(
    State(ctx): State<Arc<Context>>,
    ConnectInfo(peer): ConnectInfo<crate::transport::Peer>,
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
        return error(StatusCode::FORBIDDEN, "来源不匹配或请求过于频繁");
    }
    let Ok(slot) = ctx.slots.clone().try_acquire_owned() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "连接过多");
    };
    ws.max_message_size(8192)
        .max_frame_size(8192)
        .max_write_buffer_size(65536)
        .write_buffer_size(0)
        .on_upgrade(move |socket| async move {
            let _slot = slot;
            connection(socket, ctx).await
        })
}
fn qft_reply(
    rx: tokio::sync::oneshot::Receiver<Result<Option<crate::control::QftFrameBytes>, String>>,
) -> BoxFuture<'static, Result<Option<crate::control::QftFrameBytes>, String>> {
    Box::pin(async move {
        match tokio::time::timeout(Duration::from_secs(8), rx).await {
            Ok(Ok(result)) => result,
            _ => Err("QFT 执行确认不可用".into()),
        }
    })
}
struct StageGuard(Arc<AtomicUsize>);
impl Drop for StageGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
fn qft_stage_reply(
    rx: tokio::sync::oneshot::Receiver<Result<serde_json::Value, String>>,
    inflight: Arc<AtomicUsize>,
) -> BoxFuture<'static, Result<Option<crate::control::QftFrameBytes>, String>> {
    inflight.fetch_add(1, Ordering::Relaxed);
    let guard = StageGuard(inflight);
    Box::pin(async move {
        let _guard = guard;
        match tokio::time::timeout(Duration::from_secs(8), rx).await {
            Ok(Ok(Ok(_))) => Ok(None),
            Ok(Ok(Err(error))) => Err(error),
            _ => Err("QFT 执行确认不可用".into()),
        }
    })
}
async fn connection(mut socket: WebSocket, ctx: Arc<Context>) {
    let mut shutdown = ctx.shutdown.clone();
    let first = tokio::select! { _=shutdown.changed()=>{close(&mut socket,4000).await;return;},message=tokio::time::timeout(Duration::from_secs(3),socket.recv())=>message};
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
    let Some((id, mut cancelled, device)) = ctx.authorize(&token) else {
        close(&mut socket, 4003).await;
        return;
    };
    let preferences = ctx.store.lock().unwrap().get();
    let primary = preferences.control_mode == crate::preferences::ControlMode::Primary
        && preferences.primary_device.as_deref() == Some(&device.id);
    if !send(
        &mut socket,
        serde_json::json!({"kind":"ready","qft":crate::qft::capability(id),"deviceId":device.id,"deviceName":device.name,"permissions":device.permissions,"controlMode":preferences.control_mode,"primary":primary,"presence":ctx.presence(&device.id)}),
    )
    .await
    {
        ctx.detach(id);
        return;
    }
    let mut heartbeat = tokio::time::interval(Duration::from_millis(500));
    let mut last_alive = Instant::now();
    let mut rate = Limiter::new(100, 1);
    let mut motion_rate = Limiter::new(300, 1);
    let mut texts = Limiter::new(4, 10);
    let may_act = device.permissions.input || device.permissions.text || device.permissions.power;
    let mut qft_opened = false;
    type QftPending = BoxFuture<'static, Result<Option<crate::control::QftFrameBytes>, String>>;
    let mut pending: FuturesUnordered<QftPending> = FuturesUnordered::new();
    let stage_inflight = Arc::new(AtomicUsize::new(0));
    let mut close_code = None;
    loop {
        tokio::select! {
            biased;
            _=shutdown.changed()=>{close_code=Some(4000);break;},
            _=cancelled.changed()=>{let code=*cancelled.borrow();close_code=Some(if code==0{4001}else{code});break;},
            _=heartbeat.tick()=>{if last_alive.elapsed()>Duration::from_millis(3500){close_code=Some(4000);break;}},
            Some(result)=pending.next(),if !pending.is_empty()=>{
                match result {
                    Err(_)=>{close_code=Some(4002);break;},
                    Ok(Some(bytes))=>{
                        if !send_frame(&mut socket,Message::Binary(bytes.to_vec().into())).await {break;}
                    },
                    Ok(None)=>{}
                }
            },
            frame=socket.recv()=>{
                let Some(Ok(frame))=frame else{break;};
                if !ctx.current(id){close_code=Some(4001);break;}
                if let Message::Binary(bytes)=&frame {
                    let Ok(packet)=qfast_transmission::decode(bytes) else {close_code=Some(4002);break;};
                    let cancel=matches!(packet.body,qfast_transmission::Body::Cancel|qfast_transmission::Body::CancelAction {..});
                    if !cancel && !motion_rate.allow("motion") {close_code=Some(4008);break;}
                    let primary_ok=ctx.store.lock().unwrap().primary_allows(&device.id);
                    // Renewing an idle session has no input effect. Text/power-only
                    // clients can commit authorized payloads but cannot carry motion.
                    let allowed = match packet.body {
                        qfast_transmission::Body::Checkpoint {..} => device.permissions.input && primary_ok,
                        qfast_transmission::Body::Action {total,..} => may_act && primary_ok && (device.permissions.input || total == qfast_transmission::Vector::ZERO),
                        qfast_transmission::Body::Renew {..} => may_act,
                        qfast_transmission::Body::Cancel | qfast_transmission::Body::CancelAction {..} => true,
                        _ => false,
                    };
                    if !qft_opened || packet.session!=id || !allowed || (!cancel && pending.len()>=32) {
                        close_code=Some(4002);break;
                    }
                    last_alive = Instant::now();
                    if cancel {pending.clear();} else if matches!(packet.body,qfast_transmission::Body::Action {..}|qfast_transmission::Body::Checkpoint {..}) {ctx.record_control(&device.id);}
                    pending.push(qft_reply(ctx.controller.qft_frame(id,packet)));
                    continue;
                }
                if !rate.allow("messages"){close_code=Some(4008);break;}
                let data=match frame {
                    Message::Text(text)=>match ClientMessage::parse(&text) {Ok(data)=>data,Err(_)=>{close_code=Some(4002);break;}},
                    Message::Close(_)=>break,
                    Message::Ping(_)|Message::Pong(_)=>{last_alive=Instant::now();continue;},
                    _=>{close_code=Some(4002);break;}
                };
                match data {
                    ClientMessage::QftOpen{}=>{
                        if qft_opened || !may_act {close_code=Some(4002);break;}
                        qft_opened=true;
                        pending.push(qft_reply(ctx.controller.qft_open_for(id,device.id.clone())));
                    },
                    ClientMessage::QftStage{id:stage,command}=>{
                        let allowed=match &command {crate::protocol::Command::Text {..}=>device.permissions.text,
                            crate::protocol::Command::Shutdown {..}=>device.permissions.power,_=>device.permissions.input};
                        let primary_ok = ctx.store.lock().unwrap().primary_allows(&device.id);
                        if !qft_opened || !allowed || stage_inflight.load(Ordering::Relaxed)>=6 || pending.len()>=32
                            || !primary_ok
                            || (matches!(command,crate::protocol::Command::Text {..})&&!texts.allow("text")) {
                            close_code=Some(4002);break;
                        }
                        pending.push(qft_stage_reply(ctx.controller.qft_stage(id,stage,command), stage_inflight.clone()));
                    },
                    ClientMessage::Ping{}=>{last_alive=Instant::now();if !send(&mut socket,serde_json::json!({"kind":"pong","presence":ctx.presence(&device.id)})).await{break;}},
                    _=>{close_code=Some(4002);break;}
                }
            }
        }
    }
    ctx.detach(id);
    if let Some(code) = close_code {
        close(&mut socket, code).await;
    }
}

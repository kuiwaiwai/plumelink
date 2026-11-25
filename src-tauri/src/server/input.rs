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
use futures_util::{future::BoxFuture, stream::FuturesUnordered, SinkExt, StreamExt};
use std::{
    collections::HashSet,
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
    uri: axum::http::Uri,
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
    if !ctx.usb_allowed(&headers,&uri,true) {return ws.on_upgrade(|mut socket|async move {super::close(&mut socket,4409).await;});}
    let Ok(slot) = ctx.slots.clone().try_acquire_owned() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "连接过多");
    };
    peer.websocket_close_grace();
    ws.max_message_size(8192)
        .max_frame_size(8192)
        .max_write_buffer_size(65536)
        .write_buffer_size(0)
        .on_upgrade(move |socket| async move {
            let _slot = slot;
            connection(socket, ctx).await
        })
}
type QftResult = Result<(Option<crate::control::QftFrameBytes>, Option<qfast_transmission::Frame>), String>;
fn qft_reply(
    rx: tokio::sync::oneshot::Receiver<Result<Option<crate::control::QftFrameBytes>, String>>,
    identity: Option<qfast_transmission::Frame>,
) -> BoxFuture<'static, QftResult> {
    Box::pin(async move {
        match tokio::time::timeout(Duration::from_secs(8), rx).await {
            Ok(Ok(result)) => result.map(|bytes| (bytes, identity)),
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
) -> BoxFuture<'static, QftResult> {
    inflight.fetch_add(1, Ordering::Relaxed);
    let guard = StageGuard(inflight);
    Box::pin(async move {
        let _guard = guard;
        match tokio::time::timeout(Duration::from_secs(8), rx).await {
            Ok(Ok(Ok(_))) => Ok((None, None)),
            Ok(Ok(Err(error))) => Err(error),
            _ => Err("QFT 执行确认不可用".into()),
        }
    })
}
async fn connection(mut socket: WebSocket, ctx: Arc<Context>) {
    let mut shutdown = ctx.stop_signal();
    let mut boundary = ctx.controller.input_boundary();
    let generation = *boundary.borrow_and_update();
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
    let (id, mut cancelled, device) = match ctx.authorize(&token) {
        Ok(authorized) => authorized,
        Err(code) => { close(&mut socket, code).await; return; }
    };
    if *boundary.borrow_and_update() != generation {
        ctx.detach(id);
        close(&mut socket, 4004).await;
        return;
    }
    let preferences = ctx.store.lock().unwrap().get();
    let primary = preferences.control_mode == crate::preferences::ControlMode::Primary
        && preferences.primary_device.as_deref() == Some(&device.id);
    if !send(
        &mut socket,
        serde_json::json!({"kind":"ready","features":["media","custom-keys"],"qft":crate::qft::capability(id),"deviceId":device.id,"deviceName":device.name,"permissions":device.permissions,"controlMode":preferences.control_mode,"primary":primary,"presence":ctx.presence(&device.id)}),
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
    let mut standby = false;
    type QftPending = BoxFuture<'static, QftResult>;
    let mut pending: FuturesUnordered<QftPending> = FuturesUnordered::new();
    let mut pending_frames = HashSet::new();
    let stage_inflight = Arc::new(AtomicUsize::new(0));
    let mut close_code = None;
    let mut peer_closed = false;
    loop {
        tokio::select! {
            biased;
            _=shutdown.changed()=>{close_code=Some(4000);break;},
            _=boundary.changed()=>{close_code=Some(4004);break;},
            _=cancelled.changed()=>{let code=*cancelled.borrow();close_code=Some(if code==0{4001}else{code});break;},
            _=heartbeat.tick()=>{if last_alive.elapsed()>Duration::from_millis(3500){close_code=Some(4000);break;}},
            Some(result)=pending.next(),if !pending.is_empty()=>{
                match result {
                    Err(_)=>{close_code=Some(4002);break;},
                    Ok((bytes,identity))=>{
                        if let Some(identity)=identity {pending_frames.remove(&identity);}
                        if let Some(bytes)=bytes {
                            if let Some(failure)=ctx.controller.qft_failure(id,&bytes) {
                                if !send(&mut socket,serde_json::json!({"kind":"qftFailure","session":id.to_string(),
                                    "lease":failure.lease,"actionId":failure.action_id,"stage":failure.stage,"message":failure.message})).await {break;}
                            }
                            if !send_frame(&mut socket,Message::Binary(bytes.to_vec().into())).await {break;}
                        }
                    },
                }
            },
            frame=socket.recv()=>{
                let Some(Ok(frame))=frame else{break;};
                if !ctx.current(id){close_code=Some(4001);break;}
                if let Message::Binary(bytes)=&frame {
                    if standby {close_code=Some(4002);break;}
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
                    if !qft_opened || packet.session!=id || !allowed {
                        close_code=Some(4002);break;
                    }
                    last_alive = Instant::now();
                    // Retries share an outcome; do not queue them behind the same slow effect.
                    let identity=qfast_transmission::Frame {serial:0,..packet};
                    if pending_frames.contains(&identity) {continue;}
                    if !cancel && pending.len()>=32 {close_code=Some(4002);break;}
                    if cancel {pending.clear();pending_frames.clear();} else if matches!(packet.body,qfast_transmission::Body::Action {..}|qfast_transmission::Body::Checkpoint {..}) {ctx.record_control(&device.id);}
                    pending_frames.insert(identity);
                    pending.push(qft_reply(ctx.controller.qft_frame(id,packet),Some(identity)));
                    continue;
                }
                if !rate.allow("messages"){close_code=Some(4008);break;}
                let data=match frame {
                    Message::Text(text)=>match ClientMessage::parse(&text) {Ok(data)=>data,Err(_)=>{close_code=Some(4002);break;}},
                    Message::Close(_)=>{peer_closed=true;break;},
                    Message::Ping(_)|Message::Pong(_)=>{last_alive=Instant::now();continue;},
                    _=>{close_code=Some(4002);break;}
                };
                match data {
                    ClientMessage::Standby{}=>{
                        if standby || qft_opened {close_code=Some(4002);break;}
                        if !ctx.enter_standby(id,&device.id) {close_code=Some(4001);break;}
                        standby=true;
                        last_alive=Instant::now();
                        if !send(&mut socket,serde_json::json!({"kind":"standby"})).await {break;}
                    },
                    ClientMessage::QftOpen{}=>{
                        if standby || qft_opened || !may_act {close_code=Some(4002);break;}
                        qft_opened=true;
                        pending.push(qft_reply(ctx.controller.qft_open_for(id,device.id.clone()),None));
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
    if peer_closed {
        let _ = tokio::time::timeout(Duration::from_millis(500), socket.flush()).await;
    } else if let Some(code) = close_code {
        close(&mut socket, code).await;
    }
}

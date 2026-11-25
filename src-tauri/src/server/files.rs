use super::{error, json, Context};
use crate::file_transfer::{self, MAX_FILE_BYTES};
use axum::{
    body::{to_bytes, Body},
    extract::Request,
    http::{header, HeaderMap, Method, StatusCode},
    response::Response,
};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

const CHUNK_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(45);

pub(super) async fn resumable_upload(
    ctx: Arc<Context>,
    request: Request,
    token: String,
    owner: String,
) -> Response {
    resumable_upload_inner(ctx,request,token,owner,CHUNK_INACTIVITY_TIMEOUT).await
}
async fn resumable_upload_inner(ctx:Arc<Context>,request:Request,token:String,owner:String,inactivity:Duration)->Response {
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    if path == "/api/uploads" && method == Method::POST {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Start {
            id: String,
            name: String,
            size: u64,
        }
        let body =
            match tokio::time::timeout(Duration::from_secs(5), to_bytes(request.into_body(), 2048))
                .await
            {
                Ok(Ok(bytes)) => bytes,
                _ => return error(StatusCode::BAD_REQUEST, "传输请求无效"),
            };
        let Ok(start) = serde_json::from_slice::<Start>(&body) else {
            return error(StatusCode::BAD_REQUEST, "传输请求无效");
        };
        if !ctx.files_allowed(&token) {
            return error(StatusCode::FORBIDDEN, "文件权限已撤销");
        }
        ctx.uploads.prune(|owner| {
            ctx.store
                .lock()
                .unwrap()
                .get()
                .devices
                .iter()
                .any(|d| d.id == owner && d.permissions.files)
        });
        let device_name = ctx.store.lock().unwrap().get().devices.iter().find(|device| device.id == owner).map(|device| device.name.clone());
        return match ctx.uploads.open_from(
            &start.id,
            &owner,
            &start.name,
            start.size,
            file_transfer::receive_dir(&ctx.store),
            device_name,
        ) {
            Ok(upload) => json(
                StatusCode::OK,
                serde_json::to_value(upload.lock().await.progress()).unwrap(),
            ),
            Err(message) => error(StatusCode::BAD_REQUEST, &message),
        };
    }
    let Some(id) = path.strip_prefix("/api/uploads/") else {
        return error(StatusCode::NOT_FOUND, "传输不存在");
    };
    if method == Method::DELETE {
        return match ctx.uploads.remove(id, &owner) {
            Ok(()) => upload_status(&ctx, id, &owner),
            Err(e) => error(StatusCode::CONFLICT, &e),
        };
    }
    if method == Method::GET { return upload_status(&ctx, id, &owner); }
    let (upload, cancelled) = match ctx.uploads.get_with_cancel(id, &owner) {
        Ok(pair) => pair,
        Err(e) => return error(StatusCode::CONFLICT, &e),
    };
    let Ok(mut upload) = upload.try_lock_owned() else {
        return error(StatusCode::CONFLICT, "分块正在写入");
    };
    let mut timing = None;
    let result = if method == Method::POST {
        if !ctx.files_allowed(&token) {
            return error(StatusCode::FORBIDDEN, "文件权限已撤销");
        }
        let context=ctx.clone();let credential=token.clone();let cancellation=cancelled.clone();
        crate::resumable::finalize(upload,move || upload_authorized(&context,&credential,&cancellation)).await
    } else if method == Method::PUT {
        let Ok(_slot) = ctx.file_slots.clone().try_acquire_owned() else {
            return error(StatusCode::TOO_MANY_REQUESTS, "正在处理其他文件");
        };
        let offset = request
            .headers()
            .get("x-upload-offset")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let digest = request
            .headers()
            .get("x-chunk-sha256")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let Some(offset) = offset else {
            return error(StatusCode::BAD_REQUEST, "缺少分块位置");
        };
        let receiving = Instant::now();
        let mut source = request.into_body().into_data_stream();
        let mut buffer = Vec::new();
        let mut hash = Sha256::new();
        let mut shutdown = ctx.stop_signal();
        let deadline = tokio::time::sleep(inactivity);
        tokio::pin!(deadline);
        let mut recheck = tokio::time::interval(Duration::from_millis(250));
        let bytes = loop {
            tokio::select! {
                biased;
                _ = shutdown.changed() => {
                    drop(upload);
                    return error(StatusCode::SERVICE_UNAVAILABLE, "服务已停止，传输已取消");
                }
                _ = &mut deadline => return error(StatusCode::REQUEST_TIMEOUT, "分块接收超时"),
                _ = recheck.tick() => {
                    if cancelled.load(Ordering::Acquire) {
                        drop(upload);
                        return error(StatusCode::CONFLICT, "传输已取消");
                    }
                    if !ctx.files_allowed(&token) {
                        drop(upload);
                        let _ = ctx.uploads.remove(id, &owner);
                        return error(StatusCode::FORBIDDEN, "文件权限已撤销");
                    }
                }
                chunk = source.next() => match chunk {
                    Some(Ok(chunk)) => {
                        if buffer.len() + chunk.len() > crate::resumable::CHUNK_BYTES {
                            return error(StatusCode::BAD_REQUEST, "分块过大或连接中断");
                        }
                        for piece in chunk.chunks(64 * 1024) {
                            if let Err((code,message))=pace_upload(&ctx,&token,&cancelled,piece.len(),&mut shutdown).await {
                                return error(code,message);
                            }
                            hash.update(piece);
                            buffer.extend_from_slice(piece);
                        }
                        if !chunk.is_empty() { deadline.as_mut().reset(tokio::time::Instant::now()+inactivity); }
                    }
                    Some(Err(_)) => return error(StatusCode::BAD_REQUEST, "分块过大或连接中断"),
                    None => break buffer,
                },
            }
        };
        if cancelled.load(Ordering::Acquire) {
            return error(StatusCode::CONFLICT, "传输已取消");
        }
        if !ctx.files_allowed(&token) {
            return error(StatusCode::FORBIDDEN, "文件权限已撤销");
        }
        let received = receiving.elapsed();
        let persisting = Instant::now();
        let result = if hex::encode(hash.finalize()) != digest {
            Err("分块校验失败，未接纳此分块".into())
        } else {
            upload.write_checked(offset, &bytes,||upload_authorized(&ctx,&token,&cancelled)).await
        };
        timing = Some(format!(
            "receive;dur={:.2},persist;dur={:.2}",
            received.as_secs_f64() * 1000.0,
            persisting.elapsed().as_secs_f64() * 1000.0
        ));
        result
    } else {
        return error(StatusCode::METHOD_NOT_ALLOWED, "不支持的传输操作");
    };
    let mut response = match result {
        Ok(progress) => json(StatusCode::OK, serde_json::to_value(progress).unwrap()),
        Err(message) => error(match message.as_str(){"文件权限已撤销"=>StatusCode::FORBIDDEN,"服务已停止，传输已取消"=>StatusCode::SERVICE_UNAVAILABLE,_=>StatusCode::CONFLICT}, &message),
    };
    if let Some(value) = timing.and_then(|v| v.parse().ok()) {
        response.headers_mut().insert("server-timing", value);
    }
    response
}
fn upload_status(ctx: &Context, id: &str, owner: &str) -> Response {
    match ctx.uploads.status(id, owner) {
        Ok(progress) => json(if progress.pending() { StatusCode::ACCEPTED } else { StatusCode::OK }, serde_json::to_value(progress).unwrap()),
        Err(message) => error(StatusCode::CONFLICT, &message),
    }
}

fn upload_authorized(ctx:&Context,token:&str,cancelled:&std::sync::atomic::AtomicBool)->Result<(),String> {
    if ctx.stopped(){Err("服务已停止，传输已取消".into())}
    else if cancelled.load(Ordering::Acquire){Err("传输已取消".into())}
    else if !ctx.files_allowed(token){Err("文件权限已撤销".into())}
    else {Ok(())}
}
async fn pace_upload(ctx:&Context,token:&str,cancelled:&std::sync::atomic::AtomicBool,bytes:usize,
    shutdown:&mut super::ConnectionStop)
    ->Result<(),(StatusCode,&'static str)> {
    let paced=ctx.controller.audio.pace_bulk(bytes);tokio::pin!(paced);
    let mut check=tokio::time::interval(Duration::from_millis(25));
    loop {
        tokio::select! {
            biased;
            _=shutdown.changed()=>return Err((StatusCode::SERVICE_UNAVAILABLE,"服务已停止，传输已取消")),
            _=check.tick()=>{
                if cancelled.load(Ordering::Acquire) {return Err((StatusCode::CONFLICT,"传输已取消"));}
                if !ctx.files_allowed(token) {return Err((StatusCode::FORBIDDEN,"文件权限已撤销"));}
            },
            _=&mut paced=>{
                if cancelled.load(Ordering::Acquire) {return Err((StatusCode::CONFLICT,"传输已取消"));}
                if !ctx.files_allowed(token) {return Err((StatusCode::FORBIDDEN,"文件权限已撤销"));}
                return Ok(());
            },
        }
    }
}
async fn pace_download(ctx:&Context,token:&str,id:u64,bytes:usize)->std::io::Result<()> {
    let paced=ctx.controller.audio.pace_bulk(bytes);tokio::pin!(paced);
    let mut shutdown=ctx.stop_signal();let mut check=tokio::time::interval(Duration::from_millis(25));
    loop {
        tokio::select! {
            biased;
            _=shutdown.changed()=>return Err(std::io::Error::new(std::io::ErrorKind::ConnectionAborted,"服务已停止")),
            _=check.tick()=>if !ctx.files_allowed(token)||!ctx.files.contains(id) {return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,"文件传输已取消"));},
            _=&mut paced=>return Ok(()),
        }
    }
}

pub(super) async fn send_file(
    ctx: Arc<Context>,
    id: u64,
    token: String,
    headers: &HeaderMap,
) -> Response {
    let Ok(slot) = ctx.download_slots.clone().try_acquire_owned() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "下载任务过多，请稍后重试");
    };
    let Some((info, path)) = ctx.files.get(id) else {
        return error(StatusCode::NOT_FOUND, "文件已取消共享或不存在");
    };
    let mut file = match tokio::fs::File::open(path).await {
        Ok(v) => v,
        Err(_) => return error(StatusCode::NOT_FOUND, "文件已取消共享或不存在"),
    };
    let metadata = match file.metadata().await {
        Ok(value) if value.is_file() && value.len() <= MAX_FILE_BYTES => value,
        _ => return error(StatusCode::CONFLICT, "文件已变化，请在电脑端重新共享"),
    };
    if metadata.len() != info.size {
        return error(StatusCode::CONFLICT, "文件大小已变化，请在电脑端重新共享");
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let etag = format!("\"{id:x}-{:x}-{modified:x}\"", info.size);
    if info.size == 0 {
        if headers.get(header::RANGE).is_some() {
            return error(StatusCode::RANGE_NOT_SATISFIABLE, "下载范围超出文件");
        }
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_LENGTH, 0)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::ETAG, etag)
            .header(header::CACHE_CONTROL, "no-store")
            .header("x-content-type-options", "nosniff")
            .body(Body::empty())
            .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "无法构造响应"));
    }
    let mut offset = 0;
    let mut end = info.size - 1;
    let partial = if let Some(range) = headers.get(header::RANGE) {
        if headers
            .get(header::IF_RANGE)
            .is_some_and(|v| v.as_bytes() != etag.as_bytes())
        {
            return error(StatusCode::PRECONDITION_FAILED, "文件已变化，不能续传");
        }
        let parsed = range
            .to_str()
            .ok()
            .and_then(|r| r.strip_prefix("bytes="))
            .and_then(|r| r.split_once('-'));
        let Some((first, last)) = parsed else {
            return error(StatusCode::RANGE_NOT_SATISFIABLE, "下载范围无效");
        };
        let Ok(start) = first.parse::<u64>() else {
            return error(StatusCode::RANGE_NOT_SATISFIABLE, "下载范围无效");
        };
        offset = start;
        if !last.is_empty() {
            let Ok(value) = last.parse::<u64>() else {
                return error(StatusCode::RANGE_NOT_SATISFIABLE, "下载范围无效");
            };
            end = value;
        }
        if offset > end || end >= info.size {
            return error(StatusCode::RANGE_NOT_SATISFIABLE, "下载范围超出文件");
        }
        if file.seek(std::io::SeekFrom::Start(offset)).await.is_err() {
            return error(StatusCode::INTERNAL_SERVER_ERROR, "无法定位文件");
        }
        true
    } else {
        false
    };
    let reader =
        tokio_util::io::ReaderStream::with_capacity(file.take(end - offset + 1), 64 * 1024);
    let stream = futures_util::stream::try_unfold(
        (reader, axum::body::Bytes::new(), slot, ctx, token),
        move |(mut reader, mut pending, slot, ctx, token)| async move {
            if !ctx.files_allowed(&token) || !ctx.files.contains(id) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "文件传输已取消",
                ));
            }
            let chunk=if pending.is_empty() {reader.next().await} else {Some(Ok(std::mem::take(&mut pending)))};
            match chunk {
                Some(Ok(bytes)) => {
                    pending=bytes;
                    let count=ctx.controller.audio.bulk_quantum(pending.len());
                    let bytes=pending.split_to(count);
                    pace_download(&ctx,&token,id,bytes.len()).await?;
                    if !ctx.files_allowed(&token) || !ctx.files.contains(id) {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "文件传输已取消",
                        ));
                    }
                    Ok(Some((bytes, (reader, pending, slot, ctx, token))))
                }
                Some(Err(error)) => Err(error),
                None => Ok(None),
            }
        },
    );
    let mut response = Response::builder()
        .status(if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, end - offset + 1)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff");
    if partial {
        response = response.header(
            header::CONTENT_RANGE,
            format!("bytes {offset}-{end}/{}", info.size),
        );
    }
    response.body(Body::from_stream(stream)).unwrap()
}

#[cfg(test)]
mod pacing_tests {
    use super::*;
    struct NoInput;
    impl crate::control::Native for NoInput {
        fn execute(&mut self,_:&crate::protocol::Command)->Result<(),String>{Ok(())}
        fn release(&mut self)->Result<(),String>{Ok(())}
    }
    async fn fixture()->(super::super::Server,Arc<crate::control::Controller>,String,String,tempfile::TempDir) {
        let root=tempfile::tempdir().unwrap();let store=crate::preferences::Store::memory();
        store.lock().unwrap().update(|p|p.receive_dir=root.path().to_string_lossy().into_owned()).unwrap();
        let mut controller=crate::control::Controller::start(||Ok(Box::new(NoInput))).unwrap();
        Arc::get_mut(&mut controller).unwrap().audio=Arc::new(crate::microphone::AudioHub::with_renderer(Arc::new(|_,stream,_,ready| {
            let _=ready.send(Ok(()));while !stream.stopped(){std::thread::sleep(Duration::from_millis(1));}Ok(())
        })));
        controller.audio.start("generated","owner","fixture").unwrap();controller.audio.note_pressure();
        let server=super::super::Server::start_with_store(std::net::Ipv4Addr::LOCALHOST,0,controller.clone(),store).await.unwrap();
        let code=server.context.code();let token=server.context.pairing.lock().unwrap().pair(&code).unwrap().unwrap();
        let owner=server.context.pairing.lock().unwrap().device(&token).unwrap().id;
        (server,controller,token,owner,root)
    }
    #[tokio::test]
    async fn a_large_upload_chunk_releases_its_lock_and_slot_on_cancel_revoke_or_stop() {
        for mode in 0..3 {
            let (mut server,controller,token,owner,root)=fixture().await;let ctx=server.context.clone();let id="a".repeat(32);
            ctx.uploads.open(&id,&owner,"generated.bin",4*1024*1024,root.path().to_owned()).unwrap();
            let entered=Arc::new(tokio::sync::Notify::new());let reading=entered.clone();let bytes=vec![0x57;4*1024*1024];let digest=hex::encode(Sha256::digest(&bytes));
            let body=Body::from_stream(futures_util::stream::once(async move {reading.notify_one();Ok::<_,std::io::Error>(axum::body::Bytes::from(bytes))}));
            let request=Request::builder().method(Method::PUT).uri(format!("/api/uploads/{id}")).header("x-upload-offset","0").header("x-chunk-sha256",digest).body(body).unwrap();
            let work=tokio::spawn(resumable_upload(ctx.clone(),request,token,owner.clone()));
            tokio::time::timeout(Duration::from_secs(1),entered.notified()).await.unwrap();
            match mode {
                0=>ctx.uploads.remove(&id,&owner).unwrap(),
                1=>ctx.store.lock().unwrap().update(|p|p.devices.iter_mut().for_each(|d|d.permissions.files=false)).unwrap(),
                _=>server.stop().await,
            }
            let response=tokio::time::timeout(Duration::from_millis(350),work).await.unwrap().unwrap();
            assert_eq!(response.status(),[StatusCode::CONFLICT,StatusCode::FORBIDDEN,StatusCode::SERVICE_UNAVAILABLE][mode]);
            assert_eq!(ctx.file_slots.available_permits(),1);
            if mode!=2 {assert_eq!(ctx.uploads.get(&id,&owner).unwrap().lock().await.progress().offset,0);}
            server.stop().await;controller.shutdown();
        }
    }
    #[tokio::test]
    async fn local_pacing_does_not_consume_receive_idle_budget_and_unshared_download_is_cancelled() {
        use futures_util::FutureExt;
        let (mut server,controller,token,_,root)=fixture().await;let ctx=server.context.clone();
        let id="b".repeat(32);let bytes=vec![0x58;128*1024];let digest=hex::encode(Sha256::digest(&bytes));
        ctx.uploads.open(&id,&ctx.pairing.lock().unwrap().device(&token).unwrap().id,"paced.bin",bytes.len() as u64,root.path().to_owned()).unwrap();
        let owner=ctx.pairing.lock().unwrap().device(&token).unwrap().id;
        let request=Request::builder().method(Method::PUT).uri(format!("/api/uploads/{id}")).header("x-upload-offset","0").header("x-chunk-sha256",digest).body(Body::from(bytes)).unwrap();
        assert_eq!(resumable_upload_inner(ctx.clone(),request,token.clone(),owner.clone(),Duration::from_millis(5)).await.status(),StatusCode::OK);
        let request=Request::builder().method(Method::PUT).uri(format!("/api/uploads/{id}")).header("x-upload-offset","131072").header("x-chunk-sha256","unused")
            .body(Body::from_stream(futures_util::stream::pending::<Result<axum::body::Bytes,std::io::Error>>())).unwrap();
        assert_eq!(resumable_upload_inner(ctx.clone(),request,token.clone(),owner,Duration::from_millis(5)).await.status(),StatusCode::REQUEST_TIMEOUT);
        let file=root.path().join("generated-download.bin");std::fs::write(&file,[1,2,3]).unwrap();let shared=ctx.files.add(file).unwrap();
        let mut pending=Box::pin(pace_download(&ctx,&token,shared.id,65536));assert!(pending.as_mut().now_or_never().is_none());ctx.files.remove(shared.id);
        assert_eq!(tokio::time::timeout(Duration::from_millis(250),pending).await.unwrap().unwrap_err().kind(),std::io::ErrorKind::PermissionDenied);
        server.stop().await;controller.shutdown();
    }

    #[tokio::test]
    async fn receive_monitor_is_wired_to_start_and_survives_server_recreation_without_phone_paths() {
        use crate::receive_history::ReceiveState;
        let (mut server, controller, token, owner, root) = fixture().await;
        let ctx = server.context.clone(); let files = ctx.files.clone();
        let id = "c".repeat(32);
        let request = Request::builder().method(Method::POST).uri("/api/uploads").body(Body::from(
            serde_json::json!({"id":id,"name":"empty.bin","size":0}).to_string())).unwrap();
        let response = resumable_upload(ctx.clone(), request, token.clone(), owner.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let phone = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        assert!(phone.get("taskId").is_none()); assert!(!String::from_utf8_lossy(&body).contains(&*root.path().to_string_lossy()));
        let record = files.received.list().remove(0); assert_eq!(record.state, ReceiveState::Receiving);
        assert!(record.device_name.is_some());
        let request = Request::builder().method(Method::POST).uri(format!("/api/uploads/{id}")).body(Body::empty()).unwrap();
        assert_eq!(resumable_upload(ctx.clone(), request, token, owner).await.status(), StatusCode::OK);
        assert_eq!(files.received.list()[0].state, ReceiveState::Completed);
        let store = ctx.store.clone(); server.stop().await; drop(ctx);
        let mut restarted = super::super::Server::start_with_files(std::net::Ipv4Addr::LOCALHOST, 0,
            controller.clone(), store, files.clone()).await.unwrap();
        assert_eq!(files.received.list()[0].task_id, record.task_id);
        assert_eq!(files.received.directory(&record.task_id).unwrap(), root.path());
        restarted.stop().await; controller.shutdown();
    }
}

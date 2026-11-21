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
        return match ctx.uploads.open(
            &start.id,
            &owner,
            &start.name,
            start.size,
            file_transfer::receive_dir(&ctx.store),
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
            Ok(()) => json(StatusCode::OK, serde_json::json!({"cancelled":true})),
            Err(e) => error(StatusCode::CONFLICT, &e),
        };
    }
    let (upload, cancelled) = match ctx.uploads.get_with_cancel(id, &owner) {
        Ok(pair) => pair,
        Err(e) => return error(StatusCode::CONFLICT, &e),
    };
    let Ok(mut upload) = upload.try_lock() else {
        return error(StatusCode::CONFLICT, "分块正在写入");
    };
    let mut timing = None;
    let result = if method == Method::GET {
        Ok(upload.progress())
    } else if method == Method::POST {
        if !ctx.files_allowed(&token) {
            return error(StatusCode::FORBIDDEN, "文件权限已撤销");
        }
        upload.finish().await
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
        let mut shutdown = ctx.shutdown.clone();
        let deadline = tokio::time::sleep(CHUNK_INACTIVITY_TIMEOUT);
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
                        if !chunk.is_empty() {
                            deadline.as_mut().reset(tokio::time::Instant::now() + CHUNK_INACTIVITY_TIMEOUT);
                        }
                        if buffer.len() + chunk.len() > crate::resumable::CHUNK_BYTES {
                            return error(StatusCode::BAD_REQUEST, "分块过大或连接中断");
                        }
                        for piece in chunk.chunks(64 * 1024) {
                            ctx.controller.audio.pace_bulk(piece.len()).await;
                            hash.update(piece);
                            buffer.extend_from_slice(piece);
                        }
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
            upload.write_verified_chunk(offset, &bytes).await
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
        Err(message) => error(StatusCode::CONFLICT, &message),
    };
    if let Some(value) = timing.and_then(|v| v.parse().ok()) {
        response.headers_mut().insert("server-timing", value);
    }
    response
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
        (reader, slot, ctx, token),
        move |(mut reader, slot, ctx, token)| async move {
            if !ctx.files_allowed(&token) || !ctx.files.contains(id) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "文件传输已取消",
                ));
            }
            match reader.next().await {
                Some(Ok(bytes)) => {
                    ctx.controller.audio.pace_bulk(bytes.len()).await;
                    if !ctx.files_allowed(&token) || !ctx.files.contains(id) {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "文件传输已取消",
                        ));
                    }
                    Ok(Some((bytes, (reader, slot, ctx, token))))
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

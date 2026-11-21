use futures_util::{SinkExt, StreamExt};
use plumelink::{
    control::{Controller, Native},
    file_transfer::FileHub,
    protocol::Command,
    server::Server,
};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
struct Fake(Arc<Mutex<Vec<Command>>>);
impl Native for Fake {
    fn qft_apply(&mut self, _session: u64, v: qfast_transmission::Vector) -> Result<(), String> {
        self.execute(&Command::Motion {
            dx: v.x,
            dy: v.y,
            scroll: v.z,
        })
    }
    fn execute(&mut self, c: &Command) -> Result<(), String> {
        self.0.lock().unwrap().push(c.clone());
        Ok(())
    }
    fn release(&mut self) -> Result<(), String> {
        self.0.lock().unwrap().push(Command::Release {});
        Ok(())
    }
}
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn fixture() -> (Server, Arc<Controller>, Arc<Mutex<Vec<Command>>>) {
    let calls = Arc::new(Mutex::new(vec![]));
    let c = calls.clone();
    let controller = Controller::start(move || Ok(Box::new(Fake(c)))).unwrap();
    (
        Server::start(std::net::Ipv4Addr::LOCALHOST, 0, controller.clone())
            .await
            .unwrap(),
        controller,
        calls,
    )
}
async fn socket(s: &Server) -> Socket {
    let mut request = format!("{}/ws", s.context.origin.replace("http:", "ws:"))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", s.context.origin.parse().unwrap());
    connect_async(request).await.unwrap().0
}
async fn pair(s: &Server) -> String {
    pair_named(s, None, "测试设备").await.0
}
async fn pair_named(s: &Server, id: Option<&str>, name: &str) -> (String, String) {
    let value = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("{}/api/pair", s.context.origin))
        .header("origin", &s.context.origin)
        .json(&json!({"code":s.context.code(),"deviceId":id,"deviceName":name}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    (
        value["token"].as_str().unwrap().to_string(),
        value["deviceId"].as_str().unwrap().to_string(),
    )
}
async fn send(ws: &mut Socket, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}
async fn receive(ws: &mut Socket) -> Message {
    tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}
async fn auth(ws: &mut Socket, token: &str) {
    send(ws, json!({"kind":"auth","token":token})).await;
    let ready: Value = serde_json::from_str(&receive(ws).await.into_text().unwrap()).unwrap();
    assert_eq!(ready["kind"], "ready");
    assert!(ready.get("qfast").is_none());
    assert_eq!(ready["qft"]["version"], 2);
    assert!(ready["deviceId"].is_string());
    assert!(ready["permissions"]["input"].as_bool().unwrap());
}

async fn qft_receive(ws: &mut Socket) -> qfast_transmission::Frame {
    let Message::Binary(bytes) = receive(ws).await else {
        panic!("expected QFT2 binary response")
    };
    qfast_transmission::decode(&bytes).unwrap()
}
async fn qft_send(ws: &mut Socket, frame: qfast_transmission::Frame) {
    ws.send(Message::Binary(
        qfast_transmission::encode(frame).unwrap().to_vec().into(),
    ))
    .await
    .unwrap();
}
#[tokio::test]
async fn qft2_real_socket_self_contained_prefix_dedup_and_cancel_result() {
    use qfast_transmission::*;
    let (mut server, controller, calls) = fixture().await;
    let token = pair(&server).await;
    let mut ws = socket(&server).await;
    auth(&mut ws, &token).await;
    send(&mut ws, json!({"kind":"qftOpen"})).await;
    let grant = qft_receive(&mut ws).await;
    let mut sender = Sender::new(grant, 0, 0, plumelink::qft::config()).unwrap();
    sender.offer(Vector::new(5, -3, 120), 0).unwrap();
    sender
        .queue_action(
            Action {
                code: 1,
                argument: 1,
            },
            0,
        )
        .unwrap();
    send(
        &mut ws,
        json!({"kind":"qftStage","id":1,"command":{"type":"click","button":"left","count":1}}),
    )
    .await;
    let action = sender.poll(0, SendBudget::AVAILABLE).unwrap().unwrap();
    qft_send(&mut ws, action).await;
    assert!(matches!(
        qft_receive(&mut ws).await.body,
        Body::Ack {
            status: Status::Committed,
            ..
        }
    ));
    qft_send(&mut ws, action).await;
    assert!(matches!(
        qft_receive(&mut ws).await.body,
        Body::Ack {
            status: Status::Committed,
            ..
        }
    ));
    // Simulate loss of both action acknowledgements at the sender.
    sender.cancel(30_000).unwrap();
    qft_send(
        &mut ws,
        sender.poll(30_000, SendBudget::AVAILABLE).unwrap().unwrap(),
    )
    .await;
    sender.receive(qft_receive(&mut ws).await, 40_000).unwrap();
    assert_eq!(sender.take_outcome(), Some(Outcome::Committed(1)));
    qft_send(&mut ws, action).await;
    let _ = qft_receive(&mut ws).await;
    let effects: Vec<_> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| !matches!(c, Command::Release {}))
        .cloned()
        .collect();
    assert_eq!(
        effects,
        vec![
            Command::Motion {
                dx: 5,
                dy: -3,
                scroll: 120
            },
            Command::Click {
                button: plumelink::protocol::Button::Left,
                count: 1
            }
        ]
    );
    server.stop().await;
    controller.shutdown();
}
#[tokio::test]
async fn qft2_rejects_pre_auth_mixed_frames_wrong_session_and_denied_payload() {
    use qfast_transmission::*;
    let (mut server, controller, calls) = fixture().await;
    let mut ws = socket(&server).await;
    qft_send(
        &mut ws,
        Frame {
            session: 7,
            lease: 1,
            serial: 1,
            body: Body::Cancel,
        },
    )
    .await;
    closed(receive(&mut ws).await, 4003);
    let token = pair(&server).await;
    for variant in 0..3 {
        let mut ws = socket(&server).await;
        auth(&mut ws, &token).await;
        send(&mut ws, json!({"kind":"qftOpen"})).await;
        let grant = qft_receive(&mut ws).await;
        match variant {
            0 => ws.send(qfast_motion(1)).await.unwrap(),
            1 => {
                qft_send(
                    &mut ws,
                    Frame {
                        session: grant.session + 1,
                        serial: 1,
                        body: Body::Cancel,
                        ..grant
                    },
                )
                .await
            }
            _ => send(
                &mut ws,
                json!({"kind":"qftStage","id":1,"command":{"type":"shutdown","confirmed":true}}),
            )
            .await,
        }
        closed(receive(&mut ws).await, 4002);
    }
    assert!(calls
        .lock()
        .unwrap()
        .iter()
        .all(|c| matches!(c, Command::Release {})));
    server.stop().await;
    controller.shutdown();
}
#[tokio::test]
async fn legacy_input_and_protocol_downgrade_are_rejected() {
    let (mut server, controller, calls) = fixture().await;
    let token = pair(&server).await;
    for request in [
        json!({"kind":"qftOff"}),
        json!({"kind":"command","id":1,"command":{"type":"key","key":"space"}}),
    ] {
        let mut ws = socket(&server).await;
        auth(&mut ws, &token).await;
        send(&mut ws, request).await;
        closed(receive(&mut ws).await, 4002);
    }
    let mut ws = socket(&server).await;
    auth(&mut ws, &token).await;
    ws.send(qfast_motion(1)).await.unwrap();
    closed(receive(&mut ws).await, 4002);
    assert!(!calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| matches!(c, Command::Key { .. } | Command::Motion { .. })));
    server.stop().await;
    controller.shutdown();
}
async fn first_action(ws: &mut Socket, command: Value) {
    use qfast_transmission::*;
    send(ws, json!({"kind":"qftOpen"})).await;
    let grant = qft_receive(ws).await;
    send(ws, json!({"kind":"qftStage","id":1,"command":command})).await;
    qft_send(
        ws,
        Frame {
            body: Body::Action {
                id: 1,
                version: 0,
                total: Vector::ZERO,
                action: Action {
                    code: 1,
                    argument: 1,
                },
            },
            serial: 1,
            ..grant
        },
    )
    .await;
}
async fn committed(ws: &mut Socket) {
    assert!(matches!(
        qft_receive(ws).await.body,
        qfast_transmission::Body::Ack {
            status: qfast_transmission::Status::Committed,
            ..
        }
    ));
}
fn qfast_motion(id: u64) -> Message {
    let mut bytes = vec![0x51, 0x46, 1, 1];
    bytes.extend_from_slice(&id.to_le_bytes());
    bytes.extend_from_slice(&1_i16.to_le_bytes());
    bytes.extend_from_slice(&(-2_i16).to_le_bytes());
    bytes.extend_from_slice(&120_i16.to_le_bytes());
    bytes.extend_from_slice(&120_u16.to_le_bytes());
    Message::Binary(bytes.into())
}
#[tokio::test]
async fn qft_cannot_bypass_input_permission_or_primary_control() {
    use plumelink::preferences::{ControlMode, Store};
    let store = Store::memory();
    let calls = Arc::new(Mutex::new(vec![]));
    let clone = calls.clone();
    let controller = Controller::start(move || Ok(Box::new(Fake(clone)))).unwrap();
    let mut server = Server::start_with_store(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        controller.clone(),
        store.clone(),
    )
    .await
    .unwrap();
    let (token, id) = pair_named(&server, Some("qfast-denied"), "禁止输入").await;
    let (_, primary) = pair_named(&server, Some("qfast-primary"), "主控").await;
    store
        .lock()
        .unwrap()
        .update(|p| {
            let permissions = &mut p
                .devices
                .iter_mut()
                .find(|d| d.id == id)
                .unwrap()
                .permissions;
            permissions.input = false;
            permissions.text = false;
            permissions.power = false;
        })
        .unwrap();
    let mut ws = socket(&server).await;
    send(&mut ws, json!({"kind":"auth","token":token})).await;
    receive(&mut ws).await;
    send(&mut ws, json!({"kind":"qftOpen"})).await;
    closed(receive(&mut ws).await, 4002);
    store
        .lock()
        .unwrap()
        .update(|p| {
            p.devices
                .iter_mut()
                .find(|d| d.id == id)
                .unwrap()
                .permissions
                .input = true;
            p.control_mode = ControlMode::Primary;
            p.primary_device = Some(primary);
        })
        .unwrap();
    let mut ws = socket(&server).await;
    auth(&mut ws, &token).await;
    send(&mut ws, json!({"kind":"qftOpen"})).await;
    let grant = qft_receive(&mut ws).await;
    qft_send(
        &mut ws,
        qfast_transmission::Frame {
            serial: 1,
            body: qfast_transmission::Body::Renew {
                version: 0,
                last_action: 0,
                total: qfast_transmission::Vector::ZERO,
            },
            ..grant
        },
    )
    .await;
    assert!(
        matches!(
            qft_receive(&mut ws).await.body,
            qfast_transmission::Body::Grant { .. }
        ),
        "secondary devices may renew without producing input"
    );
    send(
        &mut ws,
        json!({"kind":"qftStage","id":1,"command":{"type":"key","key":"left"}}),
    )
    .await;
    closed(receive(&mut ws).await, 4002);
    assert!(calls
        .lock()
        .unwrap()
        .iter()
        .all(|c| matches!(c, Command::Release {})));
    server.stop().await;
    controller.shutdown();
}

#[tokio::test]
async fn ready_presence_uses_effective_primary_mode() {
    use plumelink::preferences::{ControlMode, Store};
    let store = Store::memory();
    let controller =
        Controller::start(|| Ok(Box::new(Fake(Arc::new(Mutex::new(vec![])))))).unwrap();
    let mut server = Server::start_with_store(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        controller.clone(),
        store.clone(),
    )
    .await
    .unwrap();
    let (token, id) = pair_named(&server, Some("effective-primary"), "有效主控").await;
    store
        .lock()
        .unwrap()
        .update(|p| {
            p.control_mode = ControlMode::Free;
            p.primary_device = Some(id.clone());
        })
        .unwrap();
    let mut ws = socket(&server).await;
    send(&mut ws, json!({"kind":"auth","token":token})).await;
    let ready: Value = serde_json::from_str(&receive(&mut ws).await.into_text().unwrap()).unwrap();
    assert_eq!(ready["controlMode"], "free");
    assert_eq!(ready["primary"], false);
    assert_eq!(ready["presence"]["controlMode"], "free");
    assert_eq!(ready["presence"]["primary"], false);
    assert!(ready["presence"]["primaryName"].is_null());
    server.stop().await;
    controller.shutdown();
}
#[tokio::test]
async fn text_or_power_only_qft_rejects_motion_and_ungranted_commands() {
    use plumelink::preferences::Store;
    use qfast_transmission::*;
    for text_only in [true, false] {
        let store = Store::memory();
        let calls = Arc::new(Mutex::new(vec![]));
        let recorded = calls.clone();
        let controller = Controller::start(move || Ok(Box::new(Fake(recorded)))).unwrap();
        let mut server = Server::start_with_store(
            std::net::Ipv4Addr::LOCALHOST,
            0,
            controller.clone(),
            store.clone(),
        )
        .await
        .unwrap();
        let (token, id) = pair_named(&server, None, "独立权限").await;
        store
            .lock()
            .unwrap()
            .update(|p| {
                let permissions = &mut p
                    .devices
                    .iter_mut()
                    .find(|d| d.id == id)
                    .unwrap()
                    .permissions;
                permissions.input = false;
                permissions.text = text_only;
                permissions.power = !text_only;
            })
            .unwrap();
        let command = if text_only {
            json!({"type":"text","text":"仅文字 🛰️"})
        } else {
            json!({"type":"shutdown","confirmed":true})
        };
        let mut ws = socket(&server).await;
        send(&mut ws, json!({"kind":"auth","token":token})).await;
        receive(&mut ws).await;
        first_action(&mut ws, command.clone()).await;
        committed(&mut ws).await;
        ws.close(None).await.unwrap();
        for variant in 0..3 {
            let mut ws = socket(&server).await;
            send(&mut ws, json!({"kind":"auth","token":token})).await;
            receive(&mut ws).await;
            send(&mut ws, json!({"kind":"qftOpen"})).await;
            let grant = qft_receive(&mut ws).await;
            if variant == 2 {
                send(
                    &mut ws,
                    json!({"kind":"qftStage","id":1,"command":{"type":"key","key":"enter"}}),
                )
                .await;
            } else {
                let total = Vector::new(1, 0, 0);
                let body = if variant == 0 {
                    Body::Checkpoint { version: 1, total }
                } else {
                    send(&mut ws, json!({"kind":"qftStage","id":1,"command":command})).await;
                    Body::Action {
                        id: 1,
                        version: 1,
                        total,
                        action: Action {
                            code: 1,
                            argument: 1,
                        },
                    }
                };
                qft_send(
                    &mut ws,
                    Frame {
                        body,
                        serial: 1,
                        ..grant
                    },
                )
                .await;
            }
            closed(receive(&mut ws).await, 4002);
        }
        let effects: Vec<Command> = calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| !matches!(c, Command::Release {}))
            .cloned()
            .collect();
        assert_eq!(
            effects.len(),
            1,
            "no unauthorized motion or commands execute"
        );
        assert_eq!(serde_json::to_value(&effects[0]).unwrap(), command);
        server.stop().await;
        controller.shutdown();
    }
}
fn closed(m: Message, code: u16) {
    match m {
        Message::Close(Some(f)) => assert_eq!(u16::from(f.code), code),
        _ => panic!("expected close: {m:?}"),
    }
}
#[tokio::test]
async fn websocket_origin_size_and_message_rate() {
    let (mut s, c, calls) = fixture().await;
    let mut request = format!("{}/ws", s.context.origin.replace("http:", "ws:"))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", "http://evil.test".parse().unwrap());
    let error = connect_async(request).await.unwrap_err();
    assert!(matches!(error,tokio_tungstenite::tungstenite::Error::Http(ref r) if r.status()==403));
    let mut ws = socket(&s).await;
    ws.send(Message::Text("x".repeat(9000).into()))
        .await
        .unwrap();
    // Axum may reset oversized frames without a close handshake; either must terminate with no control.
    let result = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap();
    assert!(!matches!(result, Some(Ok(Message::Text(_)))));
    let token = pair(&s).await;
    let mut ws = socket(&s).await;
    auth(&mut ws, &token).await;
    for _ in 0..101 {
        ws.feed(Message::Text(r#"{"kind":"ping"}"#.into()))
            .await
            .unwrap();
    }
    ws.flush().await.unwrap();
    loop {
        let message = receive(&mut ws).await;
        if matches!(message, Message::Close(_)) {
            closed(message, 4008);
            break;
        }
    }
    assert!(calls
        .lock()
        .unwrap()
        .iter()
        .all(|c| *c == Command::Release {}));
    s.stop().await;
    c.shutdown();
}
#[tokio::test]
async fn http_security_resources_and_pairing_rate() {
    let (mut s, c, _) = fixture().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let origin = &s.context.origin;
    let page = client.get(origin).send().await.unwrap();
    assert_eq!(page.status(), 404);
    assert!(!page.text().await.unwrap().contains("<html"));
    for path in [
        "/package.json",
        "/src-tauri/Cargo.toml",
        "/assets/..%2fCargo.toml",
    ] {
        assert_eq!(
            client
                .get(format!("{origin}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    assert_eq!(
        client
            .get(origin)
            .header("host", "evil.test")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .post(format!("{origin}/api/pair"))
            .header("origin", "http://evil.test")
            .json(&json!({"code":s.context.code()}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    for path in ["/api/status", "/api/control"] {
        assert_eq!(
            client
                .get(format!("{origin}{path}"))
                .header("origin", origin)
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
    }
    let token = pair(&s).await;
    assert_eq!(
        client
            .get(format!("{origin}/api/status"))
            .header("origin", origin)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .post(format!("{origin}/api/control"))
            .header("origin", origin)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    for _ in 0..5 {
        assert_eq!(
            client
                .post(format!("{origin}/api/pair"))
                .header("origin", origin)
                .json(&json!({"code":"wrong"}))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
    }
    assert_eq!(
        client
            .post(format!("{origin}/api/pair"))
            .header("origin", origin)
            .json(&json!({"code":s.context.code()}))
            .send()
            .await
            .unwrap()
            .status(),
        429
    );
    s.stop().await;
    c.shutdown();
}

async fn upload_resumable(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    id: &str,
    name: &str,
    bytes: &[u8],
) -> reqwest::Response {
    use sha2::{Digest, Sha256};
    let start = client
        .post(format!("{origin}/api/uploads"))
        .header("origin", origin)
        .bearer_auth(token)
        .json(&json!({"id": id, "name": name, "size": bytes.len()}))
        .send()
        .await
        .unwrap();
    if start.status() != 200 {
        return start;
    }
    let chunk = client
        .put(format!("{origin}/api/uploads/{id}"))
        .bearer_auth(token)
        .header("x-upload-offset", "0")
        .header("x-chunk-sha256", hex::encode(Sha256::digest(bytes)))
        .body(bytes.to_vec())
        .send()
        .await
        .unwrap();
    if chunk.status() != 200 {
        return chunk;
    }
    client
        .post(format!("{origin}/api/uploads/{id}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn authenticated_file_upload_list_and_download() {
    use plumelink::preferences::Store;
    use sha2::{Digest, Sha256};
    let root = std::env::temp_dir().join(format!(
        "plumelink-files-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let incoming = root.join("incoming");
    std::fs::create_dir_all(&incoming).unwrap();
    let outgoing = root.join("hello.txt");
    std::fs::write(&outgoing, b"from windows").unwrap();
    let store = Store::memory();
    store
        .lock()
        .unwrap()
        .update(|p| p.receive_dir = incoming.to_string_lossy().into_owned())
        .unwrap();
    let hub = FileHub::new();
    let shared = hub.add(outgoing.clone()).unwrap();
    let (_, controller, _) = fixture().await;
    let mut server = Server::start_with_files(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        controller.clone(),
        store,
        hub,
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let origin = server.context.origin.clone();
    let token = pair(&server).await;
    assert_eq!(
        client
            .get(format!("{origin}/api/files"))
            .header("origin", &origin)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let legacy = client
        .put(format!("{origin}/api/files/upload"))
        .bearer_auth(&token)
        .body("legacy")
        .send()
        .await
        .unwrap();
    assert_eq!(legacy.status(), 404);
    let upload = upload_resumable(
        &client,
        &origin,
        &token,
        "11111111111111111111111111111111",
        "手机照片.txt",
        b"from android",
    )
    .await;
    assert_eq!(upload.status(), 200);
    assert_eq!(
        std::fs::read(incoming.join("手机照片.txt")).unwrap(),
        b"from android"
    );
    let list = client
        .get(format!("{origin}/api/files"))
        .header("origin", &origin)
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(list["files"][0]["name"], "hello.txt");
    let bytes = client
        .get(format!("{origin}/api/files/{}", shared.id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"from windows");
    let duplicate = upload_resumable(
        &client,
        &origin,
        &token,
        "22222222222222222222222222222222",
        "手机照片.txt",
        b"second upload",
    )
    .await;
    assert_eq!(duplicate.status(), 200);
    assert_eq!(
        std::fs::read(incoming.join("手机照片.txt")).unwrap(),
        b"from android"
    );
    assert_eq!(
        std::fs::read(incoming.join("手机照片 (1).txt")).unwrap(),
        b"second upload"
    );
    let invalid = upload_resumable(
        &client,
        &origin,
        &token,
        "33333333333333333333333333333333",
        "CON.txt",
        b"denied",
    )
    .await;
    assert_eq!(invalid.status(), 400);
    std::fs::write(&outgoing, b"changed").unwrap();
    let changed = client
        .get(format!("{origin}/api/files/{}", shared.id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), 409);

    // Keep a chunked upload body open, revoke its credential, and verify cleanup before
    // the client finishes sending. This covers the in-flight authorization gap.
    let cancel_id = "44444444444444444444444444444444";
    assert_eq!(
        client
            .post(format!("{origin}/api/uploads"))
            .bearer_auth(&token)
            .json(&json!({"id": cancel_id, "name": "cancelled.txt", "size": 1000}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .put(format!("{origin}/api/uploads/{cancel_id}"))
            .bearer_auth(&token)
            .header("x-upload-offset", "0")
            .header("x-chunk-sha256", hex::encode(Sha256::digest(b"p")))
            .body("p")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut slow = tokio::net::TcpStream::connect(origin.trim_start_matches("http://"))
        .await
        .unwrap();
    slow.write_all(
        format!(
            "PUT /api/uploads/{cancel_id} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nx-upload-offset: 1\r\nx-chunk-sha256: {}\r\nContent-Length: 999\r\nConnection: close\r\n\r\npartial",
            origin.trim_start_matches("http://"),
            hex::encode(Sha256::digest(b"partial"))
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if client
                .get(format!("{origin}/api/uploads/{cancel_id}"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .status()
                == 409
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    server.context.revoke().unwrap();
    let mut response = vec![0; 4096];
    let received = tokio::time::timeout(Duration::from_secs(2), slow.read(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&response[..received]).starts_with("HTTP/1.1 403"));
    drop(slow);
    assert!(!incoming.join("cancelled.txt").exists());
    assert!(!std::fs::read_dir(&incoming).unwrap().any(|entry| entry
        .unwrap()
        .path()
        .extension()
        .is_some_and(|extension| extension == "part")));
    server.stop().await;
    controller.shutdown();
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn authentication_revocation_replay_disconnect_and_stop() {
    let (mut s, c, calls) = fixture().await;
    let mut unauth = socket(&s).await;
    send(
        &mut unauth,
        json!({"kind":"command","id":0,"command":{"type":"click","button":"left","count":1}}),
    )
    .await;
    closed(receive(&mut unauth).await, 4003);
    assert!(calls
        .lock()
        .unwrap()
        .iter()
        .all(|c| *c == Command::Release {}));
    let token = pair(&s).await;
    let mut ws = socket(&s).await;
    auth(&mut ws, &token).await;
    first_action(&mut ws, json!({"type":"click","button":"left","count":1})).await;
    committed(&mut ws).await;
    send(
        &mut ws,
        json!({"kind":"qftStage","id":1,"command":{"type":"click","button":"left","count":1}}),
    )
    .await;
    closed(receive(&mut ws).await, 4002);
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| matches!(c, Command::Click { .. }))
            .count(),
        1
    );
    assert_eq!(calls.lock().unwrap().last(), Some(&Command::Release {}));
    let mut ws = socket(&s).await;
    auth(&mut ws, &token).await;
    s.context.revoke().unwrap();
    closed(receive(&mut ws).await, 4003);
    let mut ws = socket(&s).await;
    send(&mut ws, json!({"kind":"auth","token":token})).await;
    closed(receive(&mut ws).await, 4003);
    let address = s.address;
    s.stop().await;
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    let mut replacement = Server::start(std::net::Ipv4Addr::LOCALHOST, address.port(), c.clone())
        .await
        .unwrap();
    replacement.stop().await;
    c.shutdown();
}

#[tokio::test]
async fn transfer_limits_keep_status_responsive_and_stop_cleans_pending_upload() {
    use plumelink::preferences::Store;
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).unwrap();
    let root = std::env::temp_dir().join(format!("plumelink-limits-{}", hex::encode(nonce)));
    std::fs::create_dir(&root).unwrap();
    let incoming = root.join("incoming");
    std::fs::create_dir(&incoming).unwrap();
    let source = root.join("large.bin");
    std::fs::File::create(&source)
        .unwrap()
        .set_len(32 * 1024 * 1024)
        .unwrap();
    let hub = FileHub::new();
    let shared = hub.add(source).unwrap();
    let store = Store::memory();
    store
        .lock()
        .unwrap()
        .update(|p| p.receive_dir = incoming.to_string_lossy().into_owned())
        .unwrap();
    let (mut fixture_server, controller, _) = fixture().await;
    fixture_server.stop().await;
    let mut server = Server::start_with_files(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        controller.clone(),
        store,
        hub,
    )
    .await
    .unwrap();
    let origin = &server.context.origin;
    let token = pair(&server).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let mut downloads = Vec::new();
    for _ in 0..4 {
        let download = client
            .get(format!("{origin}/api/files/{}", shared.id))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(download.status(), 200);
        downloads.push(download);
    }
    let overflow = client
        .get(format!("{origin}/api/files/{}", shared.id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(overflow.status(), 429);
    let status = client
        .get(format!("{origin}/api/status"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(status.status(), 200);
    let pending_id = "55555555555555555555555555555555";
    let second_id = "66666666666666666666666666666666";
    for (id, name) in [(pending_id, "pending.txt"), (second_id, "second.txt")] {
        assert_eq!(
            client
                .post(format!("{origin}/api/uploads"))
                .bearer_auth(&token)
                .json(&json!({"id": id, "name": name, "size": 1000}))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
    }
    assert_eq!(
        client
            .put(format!("{origin}/api/uploads/{pending_id}"))
            .bearer_auth(&token)
            .header("x-upload-offset", "0")
            .header("x-chunk-sha256", hex::encode(Sha256::digest(b"x")))
            .body("x")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let mut slow = tokio::net::TcpStream::connect(origin.trim_start_matches("http://"))
        .await
        .unwrap();
    slow.write_all(
        format!(
            "PUT /api/uploads/{pending_id} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nx-upload-offset: 1\r\nx-chunk-sha256: {}\r\nContent-Length: 999\r\n\r\nx",
            origin.trim_start_matches("http://"),
            hex::encode(Sha256::digest(b"no"))
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while client
            .get(format!("{origin}/api/uploads/{pending_id}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status()
            != 409
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let overflow = client
        .put(format!("{origin}/api/uploads/{second_id}"))
        .bearer_auth(&token)
        .header("x-upload-offset", "0")
        .header("x-chunk-sha256", hex::encode(Sha256::digest(b"no slot")))
        .body("no slot")
        .send()
        .await
        .unwrap();
    assert_eq!(overflow.status(), 429);
    drop(downloads);
    server.stop().await;
    controller.shutdown();
    assert_eq!(std::fs::read_dir(&incoming).unwrap().count(), 0);
    drop(slow);
    drop(client);
    // Dropping a tokio file schedules its Windows handle close on the blocking pool.
    // Keep the cleanup assertions above strict, but wait for those closes before
    // removing the downloaded source and its parent directory.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match std::fs::remove_dir_all(&root) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("failed to remove test directory: {error}"),
            }
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn concurrent_clients_malformed_commands_and_heartbeat() {
    let (mut s, c, calls) = fixture().await;
    let first_token = pair(&s).await;
    let second_token = pair(&s).await;
    let mut old = socket(&s).await;
    auth(&mut old, &first_token).await;
    let mut new = socket(&s).await;
    auth(&mut new, &second_token).await;
    first_action(&mut old, json!({"type":"key","key":"left"})).await;
    committed(&mut old).await;
    first_action(&mut new, json!({"type":"key","key":"right"})).await;
    committed(&mut new).await;
    send(
        &mut new,
        json!({"kind":"command","id":1,"command":{"type":"release","extra":true}}),
    )
    .await;
    closed(receive(&mut new).await, 4002);
    send(&mut old, json!({"kind":"ping"})).await;
    assert!(receive(&mut old)
        .await
        .into_text()
        .unwrap()
        .contains("pong"));
    old.close(None).await.unwrap();
    let mut new = socket(&s).await;
    auth(&mut new, &second_token).await;
    let frame = tokio::time::timeout(Duration::from_secs(5), new.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    closed(frame, 4000);
    {
        let recorded = calls.lock().unwrap();
        assert_eq!(
            recorded
                .iter()
                .filter(|command| matches!(command, Command::Key { .. }))
                .count(),
            2
        );
        assert_eq!(recorded.last(), Some(&Command::Release {}));
    }
    s.stop().await;
    c.shutdown();
}

#[tokio::test]
async fn named_devices_permissions_primary_mode_takeover_and_individual_revocation() {
    use plumelink::preferences::{ControlMode, Store};
    let store = Store::memory();
    let (_, controller, calls) = fixture().await;
    let mut server = Server::start_with_store(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        controller.clone(),
        store.clone(),
    )
    .await
    .unwrap();
    let (first_token, first_id) = pair_named(&server, Some("phone-first-01"), "客厅手机").await;
    let (second_token, second_id) = pair_named(&server, Some("tablet-second-02"), "平板电脑").await;
    store
        .lock()
        .unwrap()
        .update(|preferences| {
            preferences.control_mode = ControlMode::Primary;
            preferences.primary_device = Some(first_id.clone());
            preferences
                .devices
                .iter_mut()
                .find(|device| device.id == first_id)
                .unwrap()
                .permissions
                .power = true;
        })
        .unwrap();

    let mut first = socket(&server).await;
    auth(&mut first, &first_token).await;
    let mut second = socket(&server).await;
    auth(&mut second, &second_token).await;
    assert_eq!(server.context.connected_count(), 2);

    first_action(&mut first, json!({"type":"key","key":"left"})).await;
    committed(&mut first).await;
    send(&mut second, json!({"kind":"qftOpen"})).await;
    let _ = qft_receive(&mut second).await;
    send(
        &mut second,
        json!({"kind":"qftStage","id":1,"command":{"type":"key","key":"right"}}),
    )
    .await;
    closed(receive(&mut second).await, 4002);
    second = socket(&server).await;
    auth(&mut second, &second_token).await;
    send(&mut second, json!({"kind":"qftOpen"})).await;
    let _ = qft_receive(&mut second).await;
    send(
        &mut second,
        json!({"kind":"qftStage","id":1,"command":{"type":"shutdown","confirmed":true}}),
    )
    .await;
    closed(receive(&mut second).await, 4002);
    second = socket(&server).await;
    auth(&mut second, &second_token).await;

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for request in [
        client.get(format!("{}/api/windows", server.context.origin)),
        client
            .post(format!("{}/api/windows/switch", server.context.origin))
            .json(&json!({"hwnd": 1})),
    ] {
        let response = request
            .header("origin", &server.context.origin)
            .bearer_auth(&second_token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    }
    assert_eq!(
        client
            .post(format!("{}/api/control/request", server.context.origin))
            .header("origin", &server.context.origin)
            .bearer_auth(&second_token)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        server.context.pending_control_request(),
        Some((second_id.clone(), "平板电脑".into()))
    );

    // An authenticated device without an active WebSocket cannot submit a
    // request that the desktop could actually approve.
    second.close(None).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let offline_request = client
        .post(format!("{}/api/control/request", server.context.origin))
        .header("origin", &server.context.origin)
        .bearer_auth(&second_token)
        .send()
        .await
        .unwrap();
    assert_eq!(offline_request.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(server.context.pending_control_request(), None);

    server.context.revoke_device(&second_id).unwrap();
    send(&mut first, json!({"kind":"ping"})).await;
    assert!(receive(&mut first)
        .await
        .into_text()
        .unwrap()
        .contains("pong"));
    let mut rejected = socket(&server).await;
    send(&mut rejected, json!({"kind":"auth","token":second_token})).await;
    closed(receive(&mut rejected).await, 4003);
    assert!(calls
        .lock()
        .unwrap()
        .iter()
        .any(|command| matches!(command, Command::Key { .. })));

    server.stop().await;
    controller.shutdown();
}

#[tokio::test]
async fn saved_credential_authenticates_after_service_and_store_restart() {
    use plumelink::preferences::Store;
    let path = std::env::temp_dir().join(format!(
        "plumelink-server-{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (_, c, _) = fixture().await;
    let mut first = Server::start_with_store(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        c.clone(),
        Store::open(path.clone()).unwrap(),
    )
    .await
    .unwrap();
    let token = pair(&first).await;
    let mut ws = socket(&first).await;
    auth(&mut ws, &token).await;
    first.stop().await;
    closed(receive(&mut ws).await, 4000);
    drop(first);
    let mut second = Server::start_with_store(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        c.clone(),
        Store::open(path.clone()).unwrap(),
    )
    .await
    .unwrap();
    let mut ws = socket(&second).await;
    auth(&mut ws, &token).await;
    second.context.revoke().unwrap();
    closed(receive(&mut ws).await, 4003);
    second.stop().await;
    let mut third = Server::start_with_store(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        c.clone(),
        Store::open(path.clone()).unwrap(),
    )
    .await
    .unwrap();
    let mut ws = socket(&third).await;
    send(&mut ws, json!({"kind":"auth","token":token})).await;
    closed(receive(&mut ws).await, 4003);
    third.stop().await;
    c.shutdown();
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn production_server_uses_https_and_persists_device_identity() {
    use plumelink::preferences::Store;
    let path = std::env::temp_dir().join(format!(
        "plumelink-tls-{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (mut fixture_server, controller, _) = fixture().await;
    fixture_server.stop().await;
    let store = Store::open(path.clone()).unwrap();
    let mut server = Server::start_secure_with_store(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        controller.clone(),
        store.clone(),
    )
    .await
    .unwrap();
    assert!(server.context.origin.starts_with("https://"));
    // An idle TLS peer must not block a different client's handshake.
    let _idle_peer =
        tokio::net::TcpStream::connect(server.context.origin.trim_start_matches("https://"))
            .await
            .unwrap();
    let page = reqwest::Client::builder()
        .no_proxy()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get(&server.context.origin)
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 404);
    assert!(!page.text().await.unwrap().contains("<html"));
    let identity = store.lock().unwrap().get().tls.unwrap();
    assert_eq!(identity.address, "127.0.0.1");
    assert!(!identity.certificate.is_empty());
    assert!(!identity.private_key.is_empty());
    server.stop().await;
    controller.shutdown();
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn resumable_upload_integrity_ownership_completion_and_range_download() {
    use plumelink::preferences::Store;
    use sha2::{Digest, Sha256};
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).unwrap();
    let root = std::env::temp_dir().join(format!("plumelink-resume-{}", hex::encode(nonce)));
    std::fs::create_dir(&root).unwrap();
    let store = Store::memory();
    store
        .lock()
        .unwrap()
        .update(|p| p.receive_dir = root.to_string_lossy().into_owned())
        .unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let copy = calls.clone();
    let controller = Controller::start(move || Ok(Box::new(Fake(copy)))).unwrap();
    let hub = FileHub::new();
    let mut server = Server::start_with_files(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        controller.clone(),
        store,
        hub.clone(),
    )
    .await
    .unwrap();
    let token = pair(&server).await;
    let other = pair_named(&server, Some("another-phone"), "其他设备")
        .await
        .0;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let base = &server.context.origin;
    let id = "0123456789abcdef0123456789abcdef";
    let start = client
        .post(format!("{base}/api/uploads"))
        .bearer_auth(&token)
        .json(&json!({"id":id,"name":"resume.bin","size":6}))
        .send()
        .await
        .unwrap();
    assert_eq!(start.status(), 200);
    let path = format!("{base}/api/uploads/{id}");
    assert_eq!(
        client
            .get(&path)
            .bearer_auth(&other)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        client
            .put(&path)
            .bearer_auth(&token)
            .header("x-upload-offset", "0")
            .header("x-chunk-sha256", "bad")
            .body("abc")
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    for (offset, bytes) in [(0, "abc"), (3, "def")] {
        let response = client
            .put(&path)
            .bearer_auth(&token)
            .header("x-upload-offset", offset)
            .header("x-chunk-sha256", hex::encode(Sha256::digest(bytes)))
            .body(bytes)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let progress: Value = client
            .get(&path)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(progress["offset"], offset + 3);
    }
    for _ in 0..2 {
        let result: Value = client
            .post(&path)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(result["name"], "resume.bin");
        assert_eq!(result["complete"], true);
    }
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(std::fs::read(root.join("resume.bin")).unwrap(), b"abcdef");
    let shared = hub.add(root.join("resume.bin")).unwrap();
    let download = format!("{base}/api/files/{}", shared.id);
    let first = client
        .get(&download)
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let etag = first.headers()["etag"].to_str().unwrap().to_string();
    let range = client
        .get(&download)
        .bearer_auth(&token)
        .header("range", "bytes=3-")
        .header("if-range", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(range.status(), 206);
    assert_eq!(range.headers()["content-range"], "bytes 3-5/6");
    assert_eq!(range.bytes().await.unwrap(), &b"def"[..]);
    assert_eq!(
        client
            .get(&download)
            .bearer_auth(&token)
            .header("range", "bytes=6-")
            .send()
            .await
            .unwrap()
            .status(),
        416
    );
    assert_eq!(
        client
            .get(&download)
            .bearer_auth(&token)
            .header("range", "bytes=3-")
            .header("if-range", "changed")
            .send()
            .await
            .unwrap()
            .status(),
        412
    );
    // A sparse 2 GiB file exercises offsets beyond signed 32-bit without allocating its contents.
    let large = root.join("boundary.bin");
    {
        use std::io::{Seek, Write};
        let mut file = std::fs::File::create(&large).unwrap();
        file.set_len(2147483648).unwrap();
        file.seek(std::io::SeekFrom::Start(2147483644)).unwrap();
        file.write_all(b"tail").unwrap();
    }
    let boundary = hub.add(large.clone()).unwrap();
    assert_eq!(boundary.size, 2147483648);
    let tail = client
        .get(format!("{base}/api/files/{}", boundary.id))
        .bearer_auth(&token)
        .header("range", "bytes=2147483644-")
        .send()
        .await
        .unwrap();
    assert_eq!(tail.status(), 206);
    assert_eq!(tail.headers()["content-length"], "4");
    assert_eq!(tail.bytes().await.unwrap(), &b"tail"[..]);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&large)
        .unwrap()
        .set_len(2147483649)
        .unwrap();
    assert!(hub.add(large).is_err());
    let largeid = "1123456789abcdef0123456789abcdef";
    assert_eq!(
        client
            .post(format!("{base}/api/uploads"))
            .bearer_auth(&token)
            .json(&json!({"id":largeid,"name":"2GB.bin","size":2147483648u64}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .delete(format!("{base}/api/uploads/{largeid}"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    server.stop().await;
    controller.shutdown();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn microphone_socket_auth_audio_ack_and_permission_revocation() {
    use plumelink::{
        microphone::{AudioHub, FRAME_BYTES},
        preferences::Store,
    };
    let store = Store::memory();
    store
        .lock()
        .unwrap()
        .update(|p| p.audio_output = "test-sink".into())
        .unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let copy = calls.clone();
    let rendered = Arc::new(Mutex::new(Vec::<i16>::new()));
    let record = rendered.clone();
    let mut controller = Controller::start(move || Ok(Box::new(Fake(copy)))).unwrap();
    Arc::get_mut(&mut controller).unwrap().audio = Arc::new(AudioHub::with_renderer(Arc::new(
        move |_, stream, _, ready| {
            ready.send(Ok(())).unwrap();
            while !stream.stopped() {
                let mut buffer = [0i16; 120];
                stream.consume(&mut buffer);
                record.lock().unwrap().extend(buffer);
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        },
    )));
    let mut server = Server::start_with_store(
        std::net::Ipv4Addr::LOCALHOST,
        0,
        controller.clone(),
        store.clone(),
    )
    .await
    .unwrap();
    let (token, id) = pair_named(&server, None, "麦克风测试").await;
    let mut control = socket(&server).await;
    auth(&mut control, &token).await;
    control.close(None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        server.context.online_device_ids().is_empty(),
        "audio authenticates independently of input"
    );
    let mut request = format!(
        "{}/microphone",
        server.context.origin.replace("http:", "ws:")
    )
    .into_client_request()
    .unwrap();
    request
        .headers_mut()
        .insert("origin", server.context.origin.parse().unwrap());
    let mut mic = connect_async(request.clone()).await.unwrap().0;
    send(&mut mic, json!({"kind":"auth","token":"0".repeat(64)})).await;
    closed(receive(&mut mic).await, 4003);
    let mut mic = connect_async(request.clone()).await.unwrap().0;
    send(&mut mic, json!({"kind":"auth","token":token})).await;
    let ready: Value = serde_json::from_str(receive(&mut mic).await.to_text().unwrap()).unwrap();
    assert_eq!(ready["rate"], 24000);
    let capture_start = std::time::Instant::now();
    let mut bytes = vec![0u8; FRAME_BYTES];
    bytes[..4].copy_from_slice(b"QFM1");
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&20u32.to_le_bytes());
    for sample in bytes[12..].as_chunks_mut::<2>().0 {
        sample.copy_from_slice(&1234i16.to_le_bytes());
    }
    for sequence in 1u32..=4 {
        bytes[4..8].copy_from_slice(&sequence.to_le_bytes());
        bytes[8..12].copy_from_slice(&(sequence * 20).to_le_bytes());
        mic.send(Message::Binary(bytes.clone().into()))
            .await
            .unwrap();
        let ack: Value = serde_json::from_str(receive(&mut mic).await.to_text().unwrap()).unwrap();
        assert_eq!(ack["sequence"], sequence);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Allow actual output scheduling after the 40 ms startup buffer has filled.
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(rendered.lock().unwrap().contains(&1234));
    assert_eq!(controller.audio.status().received_frames, 4);
    assert_eq!(controller.audio.status().received_signal_frames, 4);
    assert_eq!(
        controller.audio.status().rendered_samples,
        0,
        "a simulated renderer does not claim Windows output"
    );
    // A temporary Wi-Fi stall survives, but buffered old speech must never be played on recovery.
    tokio::time::sleep(Duration::from_millis(900)).await;
    bytes[4..8].copy_from_slice(&5u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&100u32.to_le_bytes());
    for sample in bytes[12..].as_chunks_mut::<2>().0 {
        sample.copy_from_slice(&(-16_000i16).to_le_bytes());
    }
    mic.send(Message::Binary(bytes.clone().into()))
        .await
        .unwrap();
    let ack: Value = serde_json::from_str(receive(&mut mic).await.to_text().unwrap()).unwrap();
    assert_eq!(ack["sequence"], 5);
    for sequence in 6u32..=10 {
        bytes[4..8].copy_from_slice(&sequence.to_le_bytes());
        bytes[8..12]
            .copy_from_slice(&(capture_start.elapsed().as_millis() as u32 + 20).to_le_bytes());
        for sample in bytes[12..].as_chunks_mut::<2>().0 {
            sample.copy_from_slice(&2345i16.to_le_bytes());
        }
        mic.send(Message::Binary(bytes.clone().into()))
            .await
            .unwrap();
        let ack: Value = serde_json::from_str(receive(&mut mic).await.to_text().unwrap()).unwrap();
        assert_eq!(ack["sequence"], sequence);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(rendered.lock().unwrap().contains(&2345));
    assert!(rendered.lock().unwrap().iter().all(|sample| *sample >= 0));
    store
        .lock()
        .unwrap()
        .update(|p| {
            p.devices
                .iter_mut()
                .find(|d| d.id == id)
                .unwrap()
                .permissions
                .microphone = false
        })
        .unwrap();
    closed(receive(&mut mic).await, 4000);
    store
        .lock()
        .unwrap()
        .update(|p| {
            p.devices
                .iter_mut()
                .find(|d| d.id == id)
                .unwrap()
                .permissions
                .microphone = true;
        })
        .unwrap();
    let mut idle = connect_async(request.clone()).await.unwrap().0;
    send(&mut idle, json!({"kind":"auth","token":token})).await;
    let ready: Value = serde_json::from_str(receive(&mut idle).await.to_text().unwrap()).unwrap();
    assert_eq!(ready["kind"], "ready");
    let idle_start = std::time::Instant::now();
    closed(receive(&mut idle).await, 4008);
    assert!(idle_start.elapsed() >= Duration::from_millis(1400));
    let mut stale = connect_async(request).await.unwrap().0;
    send(&mut stale, json!({"kind":"auth","token":token})).await;
    let ready: Value = serde_json::from_str(receive(&mut stale).await.to_text().unwrap()).unwrap();
    assert_eq!(ready["kind"], "ready");
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&20u32.to_le_bytes());
    stale
        .send(Message::Binary(bytes.clone().into()))
        .await
        .unwrap();
    assert!(receive(&mut stale).await.is_text());
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mut stale_closed = false;
    for sequence in 2u32..=20 {
        bytes[4..8].copy_from_slice(&sequence.to_le_bytes());
        bytes[8..12].copy_from_slice(&(sequence * 20).to_le_bytes());
        stale
            .send(Message::Binary(bytes.clone().into()))
            .await
            .unwrap();
        let response = receive(&mut stale).await;
        if response.is_close() {
            closed(response, 4008);
            stale_closed = true;
            break;
        }
        assert!(response.is_text());
        if let Ok(response) = tokio::time::timeout(Duration::from_millis(90), stale.next()).await {
            closed(response.unwrap().unwrap(), 4008);
            stale_closed = true;
            break;
        }
    }
    assert!(
        stale_closed,
        "ACKs for expired audio must not keep a silent session alive"
    );
    assert_eq!(controller.audio.status().received_frames, 1);
    server.stop().await;
    controller.shutdown();
}

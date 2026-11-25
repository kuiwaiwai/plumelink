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

#[tokio::test]
async fn device_name_reports_require_auth_and_only_update_the_authenticated_phone() {
    let (mut server, controller, _) = fixture().await;
    let (token, _) = pair_named(&server, None, "2206122SC").await;
    let (other, _) = pair_named(&server, None, "Other phone").await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("{}/api/device/name", server.context.origin);
    let body = json!({"name":"My Xiaomi","model":"2206122SC"});
    assert_eq!(client.post(&url).header("origin", &server.context.origin).json(&body).send().await.unwrap().status(), 401);
    assert_eq!(client.post(&url).header("origin", &server.context.origin).bearer_auth(&token).json(&body).send().await.unwrap().json::<Value>().await.unwrap()["deviceName"], "My Xiaomi");
    let other_status = client.get(format!("{}/api/status",server.context.origin)).bearer_auth(&other).send().await.unwrap().json::<Value>().await.unwrap();
    assert_eq!(other_status["deviceName"],"Other phone");
    for invalid in [json!({"name":"bad\nname"}), json!({"name":"x".repeat(41)}), json!({"name":"phone","deviceId":"other"})] {
        assert_eq!(client.post(&url).header("origin", &server.context.origin).bearer_auth(&token).json(&invalid).send().await.unwrap().status(),400);
    }
    server.stop().await; controller.shutdown();
}

#[tokio::test]
async fn background_standby_releases_its_device_and_preserves_other_devices_and_file_access() {
    let (mut server, controller, calls) = fixture().await;
    let (token, _) = pair_named(&server, None, "Background device").await;
    let (other_token, _) = pair_named(&server, None, "Other device").await;
    let mut old = socket(&server).await;
    auth(&mut old, &token).await;
    let mut other = socket(&server).await;
    auth(&mut other, &other_token).await;
    first_action(&mut old, json!({"type":"buttonstate","button":"left","down":true})).await;
    committed(&mut old).await;
    let mut standby = socket(&server).await;
    auth(&mut standby, &token).await;
    send(&mut standby, json!({"kind":"standby"})).await;
    assert_eq!(serde_json::from_str::<Value>(receive(&mut standby).await.to_text().unwrap()).unwrap()["kind"], "standby");
    closed(receive(&mut old).await, 4001);
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let effects = calls.lock().unwrap().clone();
            let held = effects.iter().position(|command| matches!(command, Command::Buttonstate { down:true, .. })).unwrap();
            if effects[held + 1..].iter().any(|command| matches!(command, Command::Release {})) { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("background handoff must release held input without waiting for a lease timeout");
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        for ws in [&mut standby, &mut other] {
            send(ws, json!({"kind":"ping"})).await;
            assert_eq!(serde_json::from_str::<Value>(receive(ws).await.to_text().unwrap()).unwrap()["kind"], "pong");
        }
    }
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(client.get(format!("{}/api/files", server.context.origin)).bearer_auth(&token).send().await.unwrap().status(), 200);
    let mut foreground = socket(&server).await;
    auth(&mut foreground, &token).await;
    closed(receive(&mut standby).await, 4001);
    first_action(&mut foreground, json!({"type":"key","key":"enter"})).await;
    committed(&mut foreground).await;
    assert_eq!(calls.lock().unwrap().iter().filter(|command| matches!(command, Command::Buttonstate { down:true, .. })).count(), 1);
    assert_eq!(calls.lock().unwrap().iter().filter(|command| matches!(command, Command::Key { .. })).count(), 1);
    send(&mut other, json!({"kind":"ping"})).await;
    assert_eq!(serde_json::from_str::<Value>(receive(&mut other).await.to_text().unwrap()).unwrap()["kind"], "pong");
    server.stop().await; controller.shutdown();
}

#[tokio::test]
async fn background_standby_cannot_overtake_a_newer_foreground_connection() {
    let (mut server, controller, calls) = fixture().await;
    let token = pair(&server).await;
    let mut late = socket(&server).await; auth(&mut late, &token).await;
    let mut foreground = socket(&server).await; auth(&mut foreground, &token).await;
    send(&mut late, json!({"kind":"standby"})).await;
    closed(receive(&mut late).await, 4001);
    first_action(&mut foreground, json!({"type":"key","key":"enter"})).await;
    committed(&mut foreground).await;
    assert_eq!(calls.lock().unwrap().iter().filter(|command| matches!(command, Command::Key { .. })).count(), 1);
    server.stop().await; controller.shutdown();
}

#[tokio::test]
async fn background_standby_rejects_input_and_stops_when_pairing_is_revoked() {
    let (mut server, controller, calls) = fixture().await;
    let (token, device) = pair_named(&server, None, "Standby permissions").await;
    for binary in [false, true] {
        let mut standby = socket(&server).await; auth(&mut standby, &token).await;
        send(&mut standby, json!({"kind":"standby"})).await; receive(&mut standby).await;
        if binary { standby.send(qfast_motion(1)).await.unwrap(); }
        else { send(&mut standby, json!({"kind":"qftOpen"})).await; }
        closed(receive(&mut standby).await, 4002);
    }
    let mut standby = socket(&server).await; auth(&mut standby, &token).await;
    send(&mut standby, json!({"kind":"standby"})).await; receive(&mut standby).await;
    server.context.revoke_device(&device).unwrap();
    closed(receive(&mut standby).await, 4003);
    assert!(!calls.lock().unwrap().iter().any(|command| matches!(command, Command::Key { .. } | Command::Motion { .. } | Command::Buttonstate { .. })));
    server.stop().await; controller.shutdown();
}

#[tokio::test]
async fn power_boundary_closes_lan_and_usb_without_revoking_pairing_or_file_access() {
    let (mut server, controller, calls) = fixture().await;
    let address = server.ensure_usb().await.unwrap();
    let usb = format!("http://{address}");
    let access = server.usb_access_for("").unwrap().to_string();
    let token = pair(&server).await;
    let (usb_token, _) = pair_named(&server, None, "USB boundary").await;
    let open_usb = || {
        let mut request = format!("{}/ws", usb.replace("http:", "ws:")).into_client_request().unwrap();
        request.headers_mut().insert("origin", usb.parse().unwrap());
        request.headers_mut().insert("x-plumelink-usb", access.parse().unwrap());
        async move { connect_async(request).await.unwrap().0 }
    };
    let mut lan_ws = socket(&server).await; let mut usb_ws = open_usb().await;
    auth(&mut lan_ws, &token).await; auth(&mut usb_ws, &usb_token).await;
    first_action(&mut usb_ws, json!({"type":"buttonstate","button":"left","down":true})).await;
    committed(&mut usb_ws).await;
    controller.suspend_inputs();
    closed(receive(&mut lan_ws).await, 4004); closed(receive(&mut usb_ws).await, 4004);
    let mut during = open_usb().await;
    send(&mut during, json!({"kind":"auth","token":usb_token})).await;
    closed(receive(&mut during).await, 4004);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(client.get(format!("{usb}/api/files")).header("x-plumelink-usb", &access).bearer_auth(&usb_token).send().await.unwrap().status(), 200);
    let mut invalid = socket(&server).await;
    send(&mut invalid, json!({"kind":"auth","token":"invalid"})).await;
    closed(receive(&mut invalid).await, 4003);
    controller.resume_inputs(true);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !controller.input_available() { tokio::task::yield_now().await; }
    }).await.unwrap();
    let mut new_lan = socket(&server).await; let mut new_usb = open_usb().await;
    auth(&mut new_lan, &token).await; auth(&mut new_usb, &usb_token).await;
    first_action(&mut new_usb, json!({"type":"key","key":"enter"})).await;
    committed(&mut new_usb).await;
    assert_eq!(server.usb_address_for(""), Some(address));
    let effects = calls.lock().unwrap().clone();
    let held = effects.iter().position(|c| matches!(c, Command::Buttonstate { down:true, .. })).unwrap();
    let released = effects.iter().enumerate().find(|(i,c)| *i > held && matches!(c, Command::Release {})).unwrap().0;
    let entered = effects.iter().position(|c| matches!(c, Command::Key { key:plumelink::protocol::Key::Enter })).unwrap();
    assert!(held < released && released < entered);
    assert_eq!(effects.iter().filter(|c| matches!(c, Command::Key { .. })).count(), 1);
    server.stop().await; controller.shutdown();
}

#[tokio::test]
async fn usb_and_lan_share_permissions_session_ids_and_pairing_but_not_listener_shutdown() {
    let (mut server,controller,calls)=fixture().await;
    let lan=server.context.origin.clone();let usb=format!("http://{}",server.ensure_usb().await.unwrap());
    let access=server.usb_access_for("").unwrap().to_string();
    let (first,_)=pair_named(&server,None,"LAN fixture").await;let (second,_)=pair_named(&server,None,"USB fixture").await;
    let client=reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(client.get(format!("{usb}/api/status")).header("x-plumelink-usb",&access).bearer_auth(&first).send().await.unwrap().status(),200);
    assert_eq!(client.get(format!("{usb}/api/identity")).header("origin",&lan).send().await.unwrap().status(),403);
    assert_eq!(client.get(format!("{lan}/api/identity")).header("origin",&usb).send().await.unwrap().status(),403);
    assert_eq!(client.get(format!("{usb}/api/status")).header("x-plumelink-usb",&access).send().await.unwrap().status(),401);
    let mut lan_socket=socket(&server).await;
    let mut request=format!("{usb}/ws").replace("http:","ws:").into_client_request().unwrap();request.headers_mut().insert("origin",usb.parse().unwrap());
    request.headers_mut().insert("x-plumelink-usb",access.parse().unwrap());
    let mut usb_socket=connect_async(request).await.unwrap().0;
    send(&mut lan_socket,json!({"kind":"auth","token":first})).await;let lan_ready:Value=serde_json::from_str(&receive(&mut lan_socket).await.into_text().unwrap()).unwrap();
    send(&mut usb_socket,json!({"kind":"auth","token":second})).await;let usb_ready:Value=serde_json::from_str(&receive(&mut usb_socket).await.into_text().unwrap()).unwrap();
    assert_ne!(lan_ready["qft"]["session"],usb_ready["qft"]["session"]);assert_eq!(server.context.connected_count(),2);
    first_action(&mut lan_socket,json!({"type":"key","key":"left"})).await;
    let keeping_usb=tokio::spawn(async move {
        for _ in 0..10 { send(&mut usb_socket,json!({"kind":"ping"})).await;loop{let reply=receive(&mut usb_socket).await;if let Message::Text(text)=reply { if serde_json::from_str::<Value>(&text).unwrap()["kind"]=="pong" { break; } }}tokio::time::sleep(Duration::from_millis(400)).await; }
        usb_socket
    });
    server.stop_lan().await;let mut usb_socket=keeping_usb.await.unwrap();
    assert!(server.lan_address().is_none());assert_eq!(client.get(format!("{usb}/api/status")).header("x-plumelink-usb",&access).bearer_auth(&second).send().await.unwrap().status(),200);
    first_action(&mut usb_socket,json!({"type":"key","key":"right"})).await;
    tokio::time::timeout(Duration::from_secs(2),async {loop{if calls.lock().unwrap().iter().any(|c|matches!(c,Command::Key{key:plumelink::protocol::Key::Right})){break;}tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
    server.stop().await;controller.shutdown();
}

#[tokio::test]
async fn usb_phones_have_separate_shutdown_and_release_without_stopping_lan() {
    use tokio::io::{AsyncReadExt,AsyncWriteExt};
    let (mut server,controller,calls)=fixture().await;
    let first_id="a".repeat(32);let second_id="b".repeat(32);
    let first=server.ensure_usb_for(&first_id).await.unwrap();let second=server.ensure_usb_for(&second_id).await.unwrap();assert_ne!(first,second);
    let first_origin=format!("http://{first}");let second_origin=format!("http://{second}");
    let first_access=server.usb_access_for(&first_id).unwrap().to_string();let second_access=server.usb_access_for(&second_id).unwrap().to_string();
    let (first_token,_)=pair_named(&server,None,"USB A").await;let (second_token,_)=pair_named(&server,None,"USB B").await;let (lan_token,_)=pair_named(&server,None,"LAN C").await;
    let open=|origin:String,access:String|async move {let mut request=format!("{}/ws",origin.replace("http:","ws:")).into_client_request().unwrap();request.headers_mut().insert("origin",origin.parse().unwrap());request.headers_mut().insert("x-plumelink-usb",access.parse().unwrap());connect_async(request).await.unwrap().0};
    let mut first_ws=open(first_origin.clone(),first_access.clone()).await;let mut second_ws=open(second_origin.clone(),second_access.clone()).await;let mut lan_ws=socket(&server).await;
    auth(&mut first_ws,&first_token).await;auth(&mut second_ws,&second_token).await;auth(&mut lan_ws,&lan_token).await;
    assert_eq!(server.context.connected_count(),3);
    first_action(&mut first_ws,json!({"type":"buttonstate","button":"left","down":true})).await;
    committed(&mut first_ws).await;
    let releases=calls.lock().unwrap().iter().filter(|call|matches!(call,Command::Release{})).count();
    let client=reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(client.get(format!("{second_origin}/api/status")).header("x-plumelink-usb",&second_access).bearer_auth(&second_token).header("origin",&first_origin).send().await.unwrap().status(),403);
    let mut slow=tokio::net::TcpStream::connect(first).await.unwrap();
    slow.write_all(format!("POST /api/uploads HTTP/1.1\r\nHost: {first}\r\nX-PlumeLink-USB: {first_access}\r\nAuthorization: Bearer {first_token}\r\nContent-Type: application/json\r\nContent-Length: 1024\r\n\r\n{{").as_bytes()).await.unwrap();
    server.stop_usb_for(&first_id).await;
    closed(receive(&mut first_ws).await,4000);
    let ended=tokio::time::timeout(Duration::from_secs(1),slow.read(&mut [0;64])).await.unwrap();assert!(matches!(ended,Ok(0)|Err(_)));
    tokio::time::timeout(Duration::from_secs(2),async {while calls.lock().unwrap().iter().filter(|call|matches!(call,Command::Release{})).count()<=releases {tokio::task::yield_now().await;}}).await.unwrap();
    assert_eq!(server.context.connected_count(),2);assert!(server.usb_address_for(&first_id).is_none());assert_eq!(server.usb_address_for(&second_id),Some(second));
    assert!(tokio::net::TcpStream::connect(first).await.is_err());
    for (socket,token,origin) in [(&mut second_ws,&second_token,&second_origin),(&mut lan_ws,&lan_token,&server.context.origin)] {
        send(socket,json!({"kind":"ping"})).await;assert!(receive(socket).await.into_text().unwrap().contains("pong"));
        assert_eq!(client.get(format!("{origin}/api/status")).header("x-plumelink-usb",if origin.as_str()==second_origin.as_str() {second_access.as_str()}else{""}).bearer_auth(token).send().await.unwrap().status(),200);
    }
    first_action(&mut second_ws,json!({"type":"key","key":"right"})).await;committed(&mut second_ws).await;
    first_action(&mut lan_ws,json!({"type":"key","key":"left"})).await;committed(&mut lan_ws).await;
    server.stop().await;controller.shutdown();
}

#[tokio::test]
async fn public_identity_is_minimal_and_keeps_host_origin_and_other_api_checks() {
    let (server, _controller, _calls) = fixture().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("{}/api/identity", server.context.origin);
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let value = response.json::<Value>().await.unwrap();
    assert_eq!(value.as_object().unwrap().len(), 3);
    assert_eq!(value["service"], "plumelink");
    assert_eq!(value["version"], 1);
    let id = value["id"].as_str().unwrap();
    assert_eq!(id.len(), 24);
    assert!(id.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    assert_eq!(client.get(&url).header("origin", "https://unrelated.invalid").send().await.unwrap().status(), 403);
    assert_eq!(client.get(&url).header("host", "unrelated.invalid").send().await.unwrap().status(), 403);
    assert_eq!(client.post(&url).send().await.unwrap().status(), 401);
    assert_eq!(client.get(format!("{}/api/status", server.context.origin)).send().await.unwrap().status(), 401);
    assert_eq!(client.get(&url).send().await.unwrap().json::<Value>().await.unwrap()["id"], id);
}
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

#[tokio::test]
async fn client_close_is_acknowledged_with_its_reason_and_releases_input() {
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;
    for code in [1000u16, 4000] {
        let (mut server, controller, calls) = fixture().await;
        let token = pair(&server).await;
        let mut ws = socket(&server).await;
        auth(&mut ws, &token).await;
        first_action(&mut ws, json!({"type":"buttonstate","button":"left","down":true})).await;
        committed(&mut ws).await;
        ws.send(Message::Close(Some(CloseFrame { code: code.into(), reason: "input resynchronization".into() }))).await.unwrap();
        let Message::Close(Some(reply)) = receive(&mut ws).await else { panic!("missing close acknowledgement") };
        assert_eq!(u16::from(reply.code), code);
        assert_eq!(reply.reason, "input resynchronization");
        let _ = controller.probe().await;
        assert!(calls.lock().unwrap().iter().any(|command| matches!(command, Command::Release {})));
        server.stop().await;
        controller.shutdown();
    }
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
#[tokio::test]
async fn qft_failure_note_precedes_its_unknown_ack_and_never_exposes_private_errors() {
    struct Fails(&'static str);
    impl Native for Fails {
        fn execute(&mut self, _: &Command) -> Result<(), String> { Err(self.0.into()) }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
    }
    for message in ["此应用需要管理员权限，请在电脑端启动", "文件夹已移动或删除，请在电脑端重新添加", "无法打开文件夹，请在电脑端检查访问权限", "private application path"] {
        let controller = Controller::start(move || Ok(Box::new(Fails(message)))).unwrap();
        let mut server = Server::start(std::net::Ipv4Addr::LOCALHOST, 0, controller.clone()).await.unwrap();
        let token = pair(&server).await;
        let mut ws = socket(&server).await;
        auth(&mut ws, &token).await;
        first_action(&mut ws, json!({"type":"launch","id":"a".repeat(32)})).await;
        if message != "private application path" {
            let detail: Value = serde_json::from_str(&receive(&mut ws).await.into_text().unwrap()).unwrap();
            assert_eq!(detail["kind"], "qftFailure");
            assert_eq!(detail["message"], message);
            assert_eq!(detail["lease"], 1);
            assert_eq!(detail["actionId"], 1);
            assert_eq!(detail["stage"], 1);
            assert!(detail["session"].as_str().unwrap().parse::<u64>().unwrap() > 0);
        }
        assert!(matches!(qft_receive(&mut ws).await.body, qfast_transmission::Body::Ack {
            action_id: 1, status: qfast_transmission::Status::ExecutionUncertain, ..
        }));
        server.stop().await;
        controller.shutdown();
    }
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

#[tokio::test]
async fn launch_receipts_follow_qft_admission_and_are_private_to_the_authorized_device() {
    struct Queued { queue: Arc<plumelink::launcher::jobs::LaunchQueue> }
    impl Native for Queued {
        fn launch_queue(&self) -> Option<Arc<plumelink::launcher::jobs::LaunchQueue>> { Some(self.queue.clone()) }
        fn qft_action_for(&mut self, session: u64, device: Option<&str>, command: &Command) -> Result<(), String> {
            if let Command::Launch { id, request } = command { return self.queue.request(session, device, id, request.as_deref()); }
            Ok(())
        }
        fn execute(&mut self, _: &Command) -> Result<(), String> { Ok(()) }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
    }
    let store = plumelink::preferences::Store::memory();
    let mut nonce = [0u8; 16]; getrandom::fill(&mut nonce).unwrap();
    let missing = std::env::temp_dir().join(format!("plumelink-missing-{}.exe", hex::encode(nonce)));
    assert!(!missing.exists());
    store.lock().unwrap().update(|p| p.applications.push(plumelink::launcher::Entry {
        id: "a".repeat(32), name: "Configured app".into(), path: missing.to_string_lossy().into_owned(), kind: plumelink::launcher::Kind::Application,
    })).unwrap();
    let queue = plumelink::launcher::jobs::LaunchQueue::new(store.clone());
    let controller = Controller::start(move || Ok(Box::new(Queued { queue }))).unwrap();
    let mut server = Server::start_with_store(std::net::Ipv4Addr::LOCALHOST, 0, controller.clone(), store.clone()).await.unwrap();
    let (first, first_id) = pair_named(&server, Some("launch-owner"), "Owner").await;
    let (second, _) = pair_named(&server, Some("launch-other"), "Other").await;
    let client = reqwest::Client::new(); let origin = server.context.origin.clone();
    let catalog: Value = client.get(format!("{origin}/api/applications")).bearer_auth(&first).send().await.unwrap().json().await.unwrap();
    assert_eq!(catalog["launchJobs"], true);
    let request = "b".repeat(32); let url = format!("{origin}/api/applications/launches/{request}");
    assert_eq!(client.get(&url).bearer_auth(&first).send().await.unwrap().status(), 404);
    let mut ws = socket(&server).await; auth(&mut ws, &first).await;
    first_action(&mut ws, json!({"type":"launch","id":"a".repeat(32),"request":request})).await; committed(&mut ws).await;
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let response = client.get(&url).bearer_auth(&first).send().await.unwrap(); assert_eq!(response.status(), 200);
            let report: Value = response.json().await.unwrap(); if report["state"] == "failed" { break report; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.unwrap();
    assert_eq!(result["message"], "应用文件已移动或删除，请在电脑端重新添加");
    assert!(!result.to_string().contains("plumelink-missing"));
    assert_eq!(client.get(&url).bearer_auth(&second).send().await.unwrap().status(), 404);
    assert_eq!(client.get(&url).send().await.unwrap().status(), 401);
    assert_eq!(client.get(format!("{origin}/api/applications/launches/invalid")).bearer_auth(&first).send().await.unwrap().status(), 400);
    store.lock().unwrap().update(|p| p.devices.iter_mut().find(|entry| entry.id == first_id).unwrap().permissions.input = false).unwrap();
    assert_eq!(client.get(&url).bearer_auth(&first).send().await.unwrap().status(), 403);
    server.stop().await; controller.shutdown();
}
#[tokio::test]
async fn slow_qft_action_retries_do_not_fill_the_worker_queue_or_lose_its_result() {
    use qfast_transmission::*;
    struct Slow(Arc<std::sync::atomic::AtomicUsize>, std::sync::mpsc::Receiver<()>);
    impl Native for Slow {
        fn execute(&mut self, _: &Command) -> Result<(), String> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.1.recv_timeout(Duration::from_secs(5)).map_err(|_| "test executor was not released")?;
            Ok(())
        }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
    }
    for reconcile in [false, true] {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let effects = calls.clone();
        let (resume, wait) = std::sync::mpsc::channel();
        let controller = Controller::start(move || Ok(Box::new(Slow(effects, wait)))).unwrap();
        let mut server = Server::start(std::net::Ipv4Addr::LOCALHOST, 0, controller.clone()).await.unwrap();
        let token = pair(&server).await;
        let mut ws = socket(&server).await;
        auth(&mut ws, &token).await;
        send(&mut ws, json!({"kind":"qftOpen"})).await;
        let grant = qft_receive(&mut ws).await;
        send(&mut ws, json!({"kind":"qftStage","id":1,"command":{"type":"launch","id":"a".repeat(32)}})).await;
        let body = Body::Action { id: 1, version: 0, total: Vector::ZERO, action: Action { code: 1, argument: 1 } };
        for serial in 1..=60 {
            let body = if reconcile && serial >= 40 { Body::CancelAction {
                id: 1, version: 0, total: Vector::ZERO, action: Action { code: 1, argument: 1 }
            }} else { body };
            qft_send(&mut ws, Frame { serial, body, ..grant }).await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        send(&mut ws, json!({"kind":"ping"})).await;
        let pong: Value = serde_json::from_str(&receive(&mut ws).await.into_text().unwrap()).unwrap();
        assert_eq!(pong["kind"], "pong");
        resume.send(()).unwrap();
        let result = qft_receive(&mut ws).await;
        assert!(matches!(result.body, Body::Ack { action_id: 1, status: Status::Committed, .. }));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        server.stop().await;
        controller.shutdown();
    }
}

#[tokio::test]
async fn motion_burst_during_other_device_launch_preserves_release_and_disconnect() {
    use qfast_transmission::*;
    struct Input {
        calls: Arc<Mutex<Vec<(u64, Command)>>>,
        started: Option<tokio::sync::oneshot::Sender<()>>,
        resume: std::sync::mpsc::Receiver<()>,
    }
    impl Native for Input {
        fn execute(&mut self, command: &Command) -> Result<(), String> { self.execute_for(0, command) }
        fn execute_for(&mut self, session: u64, command: &Command) -> Result<(), String> {
            self.calls.lock().unwrap().push((session, command.clone()));
            if matches!(command, Command::Launch { .. }) {
                let _ = self.started.take().unwrap().send(());
                self.resume.recv_timeout(Duration::from_secs(5)).map_err(|_| "test executor was not released")?;
            }
            Ok(())
        }
        fn qft_apply(&mut self, session: u64, delta: Vector) -> Result<(), String> {
            self.execute_for(session, &Command::Motion { dx: delta.x, dy: delta.y, scroll: delta.z })
        }
        fn release_for(&mut self, session: u64) -> Result<(), String> { self.execute_for(session, &Command::Release {}) }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
    }
    for disconnected in [false, true] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (resume, held) = std::sync::mpsc::channel();
        let controller = Controller::start(move || Ok(Box::new(Input { calls: recorded, started: Some(started), resume: held }))).unwrap();
        let mut server = Server::start(std::net::Ipv4Addr::LOCALHOST, 0, controller.clone()).await.unwrap();
        let first_token = pair_named(&server, Some("slow-launch"), "Launch client").await.0;
        let second_token = pair_named(&server, Some("drag-client"), "Drag client").await.0;
        let mut first = socket(&server).await;
        let mut second = socket(&server).await;
        auth(&mut first, &first_token).await; auth(&mut second, &second_token).await;
        send(&mut first, json!({"kind":"qftOpen"})).await;
        let first_grant = qft_receive(&mut first).await;
        first_action(&mut second, json!({"type":"buttonstate","button":"left","down":true})).await;
        let press = qft_receive(&mut second).await;
        let second_session = press.session;
        send(&mut first, json!({"kind":"qftStage","id":1,"command":{"type":"launch","id":"a".repeat(32)}})).await;
        qft_send(&mut first, Frame { serial: 1, body: Body::Action { id: 1, version: 0, total: Vector::ZERO,
            action: Action { code: 1, argument: 1 } }, ..first_grant }).await;
        waiting.await.unwrap();
        for version in 1..=30 {
            qft_send(&mut second, Frame { session: second_session, lease: 1, serial: version + 1,
                body: Body::Checkpoint { version, total: Vector::new(version as i32, 0, 0) } }).await;
        }
        if disconnected {
            second.send(Message::Close(None)).await.unwrap();
            assert!(matches!(receive(&mut second).await, Message::Close(_)));
        } else {
            send(&mut second, json!({"kind":"qftStage","id":2,"command":{"type":"buttonstate","button":"left","down":false}})).await;
            qft_send(&mut second, Frame { session: second_session, lease: 1, serial: 32,
                body: Body::Action { id: 2, version: 30, total: Vector::new(30,0,0), action: Action { code: 1, argument: 2 } } }).await;
            send(&mut second, json!({"kind":"ping"})).await;
            let response = receive(&mut second).await;
            assert!(matches!(response, Message::Text(_)), "motion burst closed the connection: {response:?}");
            let pong: Value = serde_json::from_str(&response.into_text().unwrap()).unwrap();
            assert_eq!(pong["kind"], "pong");
        }
        resume.send(()).unwrap();
        committed(&mut first).await;
        if !disconnected {
            let applied = qft_receive(&mut second).await;
            assert!(matches!(applied.body, Body::Ack { version: 30, status: Status::Applied, .. }));
            committed(&mut second).await;
        }
        let _ = controller.probe().await;
        let effects = calls.lock().unwrap().clone();
        let motion: Vec<_> = effects.iter().filter(|(session, command)| *session == second_session && matches!(command, Command::Motion { .. })).collect();
        if disconnected {
            assert!(motion.is_empty());
            assert!(effects.contains(&(second_session, Command::Release {})));
        } else {
            assert_eq!(motion.len(), 1);
            assert_eq!(motion[0].1, Command::Motion { dx: 30, dy: 0, scroll: 0 });
            assert_eq!(effects.iter().filter(|(session, command)| *session == second_session && matches!(command, Command::Buttonstate { down: false, .. })).count(), 1);
        }
        server.stop().await; controller.shutdown();
    }
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
        for variant in 0..5 {
            let mut ws = socket(&server).await;
            send(&mut ws, json!({"kind":"auth","token":token})).await;
            receive(&mut ws).await;
            send(&mut ws, json!({"kind":"qftOpen"})).await;
            let grant = qft_receive(&mut ws).await;
            if variant >= 2 {
                send(
                    &mut ws,
                    json!({"kind":"qftStage","id":1,"command": if variant == 2 { json!({"type":"key","key":"enter"}) } else if variant == 3 { json!({"type":"launch","id":"a".repeat(32)}) } else { json!({"type":"media","action":"next"}) }}),
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
                == 202
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
    tokio::time::timeout(Duration::from_secs(2), async {
        while std::fs::read_dir(&incoming).unwrap().any(|entry| entry.unwrap().path().extension().is_some_and(|extension| extension == "part")) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
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
            != 202
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
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let response = client.delete(format!("{base}/api/uploads/{largeid}")).bearer_auth(&token).send().await.unwrap();
            let status = response.status(); let state: Value = response.json().await.unwrap();
            if status == 200 { assert_eq!(state["cancelled"], true); break; }
            assert_eq!(status, 202); assert_eq!(state["cancelled"], false);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    server.stop().await;
    controller.shutdown();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn microphone_initialization_keeps_heartbeats_and_cancelled_sessions_owned() {
    use plumelink::{microphone::{AudioHub, FRAME_BYTES}, preferences::Store};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let release_first = Arc::new(AtomicBool::new(false));
    struct Unblock(Arc<AtomicBool>);
    impl Drop for Unblock { fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); } }
    let _unblock = Unblock(release_first.clone());
    let entered = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let (gate, starts, cancelled_copy, finished_copy) = (release_first.clone(), entered.clone(), cancelled.clone(), finished.clone());
    let store = Store::memory();
    store.lock().unwrap().update(|prefs| prefs.audio_output = "test-sink".into()).unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let mut controller = Controller::start(move || Ok(Box::new(Fake(calls)))).unwrap();
    Arc::get_mut(&mut controller).unwrap().audio = Arc::new(AudioHub::with_renderer(Arc::new(move |_, stream, _, ready| {
        let first = starts.fetch_add(1, Ordering::SeqCst) == 0;
        if first {
            let began = std::time::Instant::now();
            while !gate.load(Ordering::SeqCst) && began.elapsed() < Duration::from_secs(3) {
                if stream.stopped() { cancelled_copy.store(true, Ordering::SeqCst); }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let _ = ready.send(Ok(()));
        while !stream.stopped() { std::thread::sleep(Duration::from_millis(5)); }
        if first { finished_copy.store(true, Ordering::SeqCst); }
        Ok(())
    })));
    let mut server = Server::start_with_store(std::net::Ipv4Addr::LOCALHOST, 0, controller.clone(), store).await.unwrap();
    let token = pair(&server).await;
    let mut request = format!("{}/microphone", server.context.origin.replace("http:", "ws:")).into_client_request().unwrap();
    request.headers_mut().insert("origin", server.context.origin.parse().unwrap());
    let mut first = connect_async(request.clone()).await.unwrap().0;
    send(&mut first, json!({"kind":"auth","token":token})).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while entered.load(Ordering::SeqCst) == 0 { tokio::time::sleep(Duration::from_millis(5)).await; }
    }).await.unwrap();
    first.send(Message::Ping(b"initializing".to_vec().into())).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_millis(400), first.next()).await.expect("device initialization must not block heartbeat replies").unwrap().unwrap();
    assert!(reply.is_pong());
    first.close(None).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !cancelled.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(5)).await; }
    }).await.expect("cancelling initialization must release its logical session");
    let mut next = connect_async(request).await.unwrap().0;
    send(&mut next, json!({"kind":"auth","token":token})).await;
    let ready: Value = serde_json::from_str(receive(&mut next).await.to_text().unwrap()).unwrap();
    assert_eq!(ready["kind"], "ready");
    release_first.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(1), async {
        while !finished.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(5)).await; }
    }).await.unwrap();
    let mut frame = vec![0u8; FRAME_BYTES];
    frame[..4].copy_from_slice(b"QFM1");
    frame[4..8].copy_from_slice(&1u32.to_le_bytes());
    frame[8..12].copy_from_slice(&20u32.to_le_bytes());
    next.send(Message::Binary(frame.into())).await.unwrap();
    let ack: Value = serde_json::from_str(receive(&mut next).await.to_text().unwrap()).unwrap();
    assert_eq!(ack["sequence"], 1, "late completion of a cancelled initializer must not stop its replacement");
    assert_eq!(controller.audio.status().received_frames, 1);
    next.close(None).await.unwrap();
    server.stop().await;
    controller.shutdown();
}

#[tokio::test]
async fn microphone_initialization_keeps_driver_errors_and_rechecks_revoked_permission() {
    use plumelink::{microphone::AudioHub, preferences::Store};
    use std::sync::atomic::{AtomicBool, Ordering};
    for (revoke, reports_ready) in [(false, true), (false, false), (true, true)] {
        let release = Arc::new(AtomicBool::new(false));
        struct Unblock(Arc<AtomicBool>);
        impl Drop for Unblock { fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); } }
        let _unblock = Unblock(release.clone());
        let entered = Arc::new(AtomicBool::new(false));
        let (gate, started) = (release.clone(), entered.clone());
        let store = Store::memory();
        store.lock().unwrap().update(|prefs| prefs.audio_output = "test-sink".into()).unwrap();
        let calls = Arc::new(Mutex::new(vec![]));
        let mut controller = Controller::start(move || Ok(Box::new(Fake(calls)))).unwrap();
        Arc::get_mut(&mut controller).unwrap().audio = Arc::new(AudioHub::with_renderer(Arc::new(move |_, _, _, ready| {
            started.store(true, Ordering::SeqCst);
            let began = std::time::Instant::now();
            while !gate.load(Ordering::SeqCst) && began.elapsed() < Duration::from_secs(3) {
                std::thread::sleep(Duration::from_millis(1));
            }
            if reports_ready { let _ = ready.send(Err("driver startup test".into())); }
            Err("driver startup test".into())
        })));
        let mut server = Server::start_with_store(std::net::Ipv4Addr::LOCALHOST, 0, controller.clone(), store.clone()).await.unwrap();
        let (token, id) = pair_named(&server, None, "初始化检查").await;
        let mut request = format!("{}/microphone", server.context.origin.replace("http:", "ws:")).into_client_request().unwrap();
        request.headers_mut().insert("origin", server.context.origin.parse().unwrap());
        let mut mic = connect_async(request).await.unwrap().0;
        send(&mut mic, json!({"kind":"auth","token":token})).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while !entered.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(5)).await; }
        }).await.unwrap();
        if revoke {
            store.lock().unwrap().update(|prefs| prefs.devices.iter_mut().find(|device| device.id == id).unwrap().permissions.microphone = false).unwrap();
        }
        release.store(true, Ordering::SeqCst);
        let response = receive(&mut mic).await;
        if revoke { closed(response, 4403); }
        else {
            let error: Value = serde_json::from_str(response.to_text().unwrap()).unwrap();
            assert_eq!(error["kind"], "error");
            assert_eq!(error["message"], "driver startup test");
            closed(receive(&mut mic).await, 4002);
            assert_eq!(controller.audio.status().error, "driver startup test");
        }
        server.stop().await;
        controller.shutdown();
    }
}

#[tokio::test]
async fn microphone_allows_preparation_then_applies_live_idle_and_cancel_deadlines() {
    use plumelink::{microphone::{AudioHub, FRAME_BYTES}, preferences::Store};
    let store = Store::memory();
    store.lock().unwrap().update(|prefs| prefs.audio_output = "test-sink".into()).unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let mut controller = Controller::start(move || Ok(Box::new(Fake(calls)))).unwrap();
    Arc::get_mut(&mut controller).unwrap().audio = Arc::new(AudioHub::with_renderer(Arc::new(
        |_, stream, _, ready| {
            ready.send(Ok(())).unwrap();
            while !stream.stopped() { std::thread::sleep(Duration::from_millis(5)); }
            Ok(())
        },
    )));
    let mut server = Server::start_with_store(std::net::Ipv4Addr::LOCALHOST, 0, controller.clone(), store.clone()).await.unwrap();
    let (token, id) = pair_named(&server, None, "慢启动麦克风").await;
    let mut request = format!("{}/microphone", server.context.origin.replace("http:", "ws:")).into_client_request().unwrap();
    request.headers_mut().insert("origin", server.context.origin.parse().unwrap());
    let mut mic = connect_async(request.clone()).await.unwrap().0;
    send(&mut mic, json!({"kind":"auth","token":token})).await;
    let ready: Value = serde_json::from_str(receive(&mut mic).await.to_text().unwrap()).unwrap();
    assert_eq!(ready["kind"], "ready");
    let premature = tokio::time::timeout(Duration::from_millis(1800), mic.next()).await;
    assert!(premature.is_err(), "capture preparation must not be closed by the live idle budget: {premature:?}");
    let mut frame = vec![0u8; FRAME_BYTES];
    frame[..4].copy_from_slice(b"QFM1");
    frame[4..8].copy_from_slice(&1u32.to_le_bytes());
    frame[8..12].copy_from_slice(&20u32.to_le_bytes());
    mic.send(Message::Binary(frame.into())).await.unwrap();
    let ack: Value = serde_json::from_str(receive(&mut mic).await.to_text().unwrap()).unwrap();
    assert_eq!(ack["sequence"], 1);
    assert_eq!(controller.audio.status().received_frames, 1);
    let live_idle = std::time::Instant::now();
    closed(receive(&mut mic).await, 4008);
    assert!(live_idle.elapsed() >= Duration::from_millis(1400));
    for revoke in [false, true] {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let mut waiting = loop {
            let mut candidate = connect_async(request.clone()).await.unwrap().0;
            send(&mut candidate, json!({"kind":"auth","token":token})).await;
            let ready: Value = serde_json::from_str(receive(&mut candidate).await.to_text().unwrap()).unwrap();
            if ready["kind"] == "ready" { break candidate; }
            assert_eq!(ready["message"], "电脑音频设备正在停止，请稍后重试", "unexpected readiness response: {ready}");
            assert!(std::time::Instant::now() < deadline, "renderer cleanup did not finish");
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        if revoke {
            store.lock().unwrap().update(|prefs| prefs.devices.iter_mut().find(|device| device.id == id).unwrap().permissions.microphone = false).unwrap();
        } else { controller.audio.stop(); }
        closed(receive(&mut waiting).await, if revoke {4403}else{4000});
    }
    server.stop().await;
    controller.shutdown();
}

#[tokio::test]
async fn microphone_status_requires_permission_and_isolates_paired_devices() {
    use plumelink::{microphone::AudioHub, preferences::Store};
    let store = Store::memory();
    store.lock().unwrap().update(|p| {
        p.audio_output = "test-sink".into(); p.audio_output_name = "Test output".into(); p.microphone_gain_db = 24;
    }).unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let mut controller = Controller::start(move || Ok(Box::new(Fake(calls)))).unwrap();
    Arc::get_mut(&mut controller).unwrap().audio = Arc::new(AudioHub::with_renderer(Arc::new(|_, stream, _, ready| {
        ready.send(Ok(())).unwrap();
        while !stream.stopped() { std::thread::sleep(Duration::from_millis(5)); }
        Ok(())
    })));
    let mut server = Server::start_with_store(std::net::Ipv4Addr::LOCALHOST, 0, controller.clone(), store.clone()).await.unwrap();
    let (owner_token, owner) = pair_named(&server, Some("status-owner"), "Owner").await;
    let (other_token, other) = pair_named(&server, Some("status-other"), "Other").await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let get = |token: String| client.get(format!("{}/api/microphone/status", server.context.origin))
        .header("origin", &server.context.origin).bearer_auth(token).send();
    let stream = controller.audio.start("test-sink", &owner, "Owner").unwrap();
    stream.push(&1234i16.to_le_bytes().repeat(480), 0, std::time::Instant::now());
    let own: Value = get(owner_token.clone()).await.unwrap().json().await.unwrap();
    assert_eq!(own["configured"], true); assert_eq!(own["gainDb"], 24); assert_eq!(own["output"], "Test output");
    assert_eq!(own["owned"], true); assert_eq!(own["status"]["receivedFrames"], 1);
    let foreign: Value = get(other_token.clone()).await.unwrap().json().await.unwrap();
    assert_eq!(foreign["owned"], false); assert_eq!(foreign["busy"], true);
    assert_eq!(foreign["status"]["receivedFrames"], 0); assert_eq!(foreign["status"]["deviceName"], "");
    stream.stop();
    let stopped: Value = get(other_token.clone()).await.unwrap().json().await.unwrap();
    assert_eq!(stopped["busy"], false);
    store.lock().unwrap().update(|p| p.devices.iter_mut().find(|d| d.id == other).unwrap().permissions.microphone = false).unwrap();
    let denied = get(other_token).await.unwrap(); assert_eq!(denied.status(), 403);
    let denied: Value = denied.json().await.unwrap(); assert!(denied.get("status").is_none()); assert!(denied.get("output").is_none());
    server.stop().await; controller.shutdown();
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
        .update(|p| {
            p.audio_output = "test-sink".into();
            p.microphone_gain_db = 12;
        })
        .unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let copy = calls.clone();
    let rendered = Arc::new(Mutex::new(Vec::<i16>::new()));
    let record = rendered.clone();
    let amplified = Arc::new(Mutex::new(Vec::<i16>::new()));
    let gained = amplified.clone();
    let mut controller = Controller::start(move || Ok(Box::new(Fake(copy)))).unwrap();
    Arc::get_mut(&mut controller).unwrap().audio = Arc::new(AudioHub::with_renderer(Arc::new(
        move |_, stream, _, ready| {
            ready.send(Ok(())).unwrap();
            while !stream.stopped() {
                let mut buffer = [0i16; 120];
                stream.consume(&mut buffer);
                record.lock().unwrap().extend(buffer);
                stream.apply_gain(&mut buffer);
                gained.lock().unwrap().extend(buffer);
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
    assert!(amplified.lock().unwrap().contains(&4913), "saved +12 dB applies to received PCM on connection");
    assert_eq!(controller.audio.status().received_frames, 4);
    assert_eq!(controller.audio.status().received_signal_frames, 4);
    assert_eq!(
        controller.audio.status().rendered_samples,
        0,
        "a simulated renderer does not claim Windows output"
    );
    {
        let mut preferences = store.lock().unwrap();
        preferences.update(|p| p.microphone_gain_db = 0).unwrap();
        controller.audio.set_gain_db(0);
    }
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
    assert!(amplified.lock().unwrap().contains(&2345), "gain updates apply to an existing capture without reconnecting");
    assert!(rendered.lock().unwrap().iter().all(|sample| *sample >= 0));
    assert_eq!(controller.audio.status().gain_clipped_samples, 0);
    controller.audio.set_gain_db(36);
    for sequence in 11u32..=14 {
        bytes[4..8].copy_from_slice(&sequence.to_le_bytes());
        bytes[8..12].copy_from_slice(&(capture_start.elapsed().as_millis() as u32 + 20).to_le_bytes());
        for sample in bytes[12..].as_chunks_mut::<2>().0 { sample.copy_from_slice(&25000i16.to_le_bytes()); }
        mic.send(Message::Binary(bytes.clone().into())).await.unwrap();
        let ack: Value = serde_json::from_str(receive(&mut mic).await.to_text().unwrap()).unwrap();
        assert_eq!(ack["sequence"], sequence);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while controller.audio.status().gain_clipped_samples == 0 { tokio::time::sleep(Duration::from_millis(5)).await; }
    }).await.expect("received loud PCM must report software gain clipping");
    assert!(amplified.lock().unwrap().contains(&i16::MAX));
    controller.audio.set_gain_db(0);
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
    closed(receive(&mut mic).await, 4403);
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
    assert_eq!(controller.audio.status().gain_clipped_samples, 0, "a new audio session starts without an old overload warning");
    let idle_start = std::time::Instant::now();
    let closed_idle = tokio::time::timeout(Duration::from_secs(7), idle.next()).await.unwrap().unwrap().unwrap();
    closed(closed_idle, 4008);
    assert!(idle_start.elapsed() >= Duration::from_millis(4500));
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

#[tokio::test]
async fn application_catalog_requires_input_and_never_returns_local_paths() {
    let store = plumelink::preferences::Store::memory();
    store.lock().unwrap().update(|p| p.applications.push(plumelink::launcher::Entry {
        id: "a".repeat(32), name: "Sample App".into(), path: "private-local-target.exe".into(), kind: plumelink::launcher::Kind::Application,
    })).unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let recorded = calls.clone();
    let controller = Controller::start(move || Ok(Box::new(Fake(recorded)))).unwrap();
    let mut server = Server::start_with_files(std::net::Ipv4Addr::LOCALHOST, 0, controller.clone(), store.clone(), FileHub::new()).await.unwrap();
    let client = reqwest::Client::new();
    let endpoint = format!("{}/api/applications", server.context.origin);
    let icon_endpoint = format!("{}/api/applications/icons", server.context.origin);
    assert_eq!(client.get(&endpoint).send().await.unwrap().status(), 401);
    assert_eq!(client.get(&icon_endpoint).send().await.unwrap().status(), 401);
    let token = pair(&server).await;
    let result: Value = client.get(&endpoint).bearer_auth(&token).send().await.unwrap().json().await.unwrap();
    assert_eq!(result["applications"][0]["name"], "Sample App");
    assert!(result["applications"][0].get("path").is_none());
    assert_eq!(result["applications"][0]["available"], false);
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).unwrap();
    let folder = std::env::temp_dir().join(format!("plumelink-shortcut-http-{}", hex::encode(nonce)));
    std::fs::create_dir(&folder).unwrap();
    plumelink::launcher::add_folder(&store, folder.clone()).unwrap();
    let catalog: Value = client.get(&endpoint).bearer_auth(&token).send().await.unwrap().json().await.unwrap();
    assert_eq!(catalog["applications"][0]["kind"], "application");
    assert_eq!(catalog["applications"][1]["kind"], "folder");
    assert_eq!(catalog["applications"][1]["available"], true);
    assert!(catalog["applications"][1].get("path").is_none());
    assert!(!catalog.to_string().contains(&folder.to_string_lossy().to_string()));
    let icons: Value = client.get(&icon_endpoint).bearer_auth(&token).send().await.unwrap().json().await.unwrap();
    assert_eq!(icons["icons"], json!([]));
    assert!(!icons.to_string().contains("private-local-target"));
    plumelink::launcher::rename(&store, &"a".repeat(32), "Renamed").unwrap();
    let renamed: Value = client.get(&endpoint).bearer_auth(&token).send().await.unwrap().json().await.unwrap();
    assert_eq!(renamed["applications"][0]["name"], "Renamed");
    assert_eq!(renamed["applications"][0]["id"], result["applications"][0]["id"]);
    assert!(!renamed.to_string().contains("private-local-target"));
    store.lock().unwrap().update(|p| p.devices[0].permissions.input = false).unwrap();
    assert_eq!(client.get(&endpoint).bearer_auth(&token).send().await.unwrap().status(), 403);
    assert_eq!(client.get(&icon_endpoint).bearer_auth(&token).send().await.unwrap().status(), 403);
    server.stop().await;
    controller.shutdown();
    std::fs::remove_dir(folder).unwrap();
}

#[tokio::test]
async fn window_query_rechecks_input_primary_and_pairing_after_background_work() {
    use plumelink::control::WindowReader;
    struct Reader(WindowReader);
    impl Native for Reader {
        fn window_reader(&self) -> Option<WindowReader> { Some(self.0.clone()) }
        fn execute(&mut self, _: &Command) -> Result<(), String> { Ok(()) }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
    }
    for change in ["input", "primary", "pairing"] {
        let (started, ready) = std::sync::mpsc::sync_channel(1);
        let (resume, wait) = std::sync::mpsc::channel();
        let wait = Mutex::new(wait);
        let read: WindowReader = Arc::new(move || {
            let _ = started.send(());
            wait.lock().unwrap().recv_timeout(std::time::Duration::from_secs(5)).unwrap();
            Ok(json!([{"hwnd":42,"title":"Private sample","app":"sample.exe","icon":null,"minimized":false}]))
        });
        let controller = Controller::start(move || Ok(Box::new(Reader(read)))).unwrap();
        let store = plumelink::preferences::Store::memory();
        let mut server = Server::start_with_files(
            std::net::Ipv4Addr::LOCALHOST, 0, controller.clone(), store.clone(), FileHub::new(),
        ).await.unwrap();
        let token = pair(&server).await;
        let endpoint = format!("{}/api/windows", server.context.origin);
        let pending = tokio::spawn(async move { reqwest::Client::new().get(endpoint).bearer_auth(token).send().await.unwrap() });
        tokio::task::spawn_blocking(move || ready.recv().unwrap()).await.unwrap();
        store.lock().unwrap().update(|p| match change {
            "input" => p.devices[0].permissions.input = false,
            "primary" => { p.control_mode = plumelink::preferences::ControlMode::Primary; p.primary_device = Some("another-device".into()); }
            _ => p.devices.clear(),
        }).unwrap();
        resume.send(()).unwrap();
        let response = pending.await.unwrap();
        assert_eq!(response.status().as_u16(), if change == "pairing" {401} else {403}, "{change}");
        assert!(!response.text().await.unwrap().contains("Private sample"));
        server.stop().await;
        controller.shutdown();
    }
}

#[tokio::test]
async fn upload_cancellation_retains_completion_and_blocks_late_creation() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let root=tempfile::tempdir().unwrap();let store=plumelink::preferences::Store::memory();
    store.lock().unwrap().update(|p|p.receive_dir=root.path().to_string_lossy().into_owned()).unwrap();
    let controller=Controller::start(||Ok(Box::new(Fake(Arc::new(Mutex::new(vec![])))))).unwrap();
    let mut server=Server::start_with_files(std::net::Ipv4Addr::LOCALHOST,0,controller.clone(),store,FileHub::new()).await.unwrap();
    let origin=server.context.origin.clone();let token=pair(&server).await;let other=pair(&server).await;
    let client=reqwest::Client::builder().no_proxy().build().unwrap();
    let delayed="6".repeat(32);let body=json!({"id":delayed,"name":"late.bin","size":1}).to_string();
    let mut request=tokio::net::TcpStream::connect(origin.trim_start_matches("http://")).await.unwrap();
    request.write_all(format!("POST /api/uploads HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{{",origin.trim_start_matches("http://"),body.len()).as_bytes()).await.unwrap();
    let cancelled:Value=client.delete(format!("{origin}/api/uploads/{delayed}")).bearer_auth(&token).send().await.unwrap().json().await.unwrap();
    assert_eq!(cancelled["state"],"cancelled");assert_eq!(cancelled["cancelled"],true);
    request.write_all(&body.as_bytes()[1..]).await.unwrap();let mut response=Vec::new();request.read_to_end(&mut response).await.unwrap();assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 400"));
    assert_eq!(client.delete(format!("{origin}/api/uploads/{delayed}")).bearer_auth(&other).send().await.unwrap().status(),409);
    let writing="8".repeat(32);
    assert_eq!(client.post(format!("{origin}/api/uploads")).bearer_auth(&token).json(&json!({"id":writing,"name":"cancelled.bin","size":100})).send().await.unwrap().status(),200);
    let mut chunk=tokio::net::TcpStream::connect(origin.trim_start_matches("http://")).await.unwrap();
    chunk.write_all(format!("PUT /api/uploads/{writing} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nX-Upload-Offset: 0\r\nX-Chunk-Sha256: {}\r\nContent-Length: 100\r\nConnection: close\r\n\r\nx",origin.trim_start_matches("http://"),"0".repeat(64)).as_bytes()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2),async {loop {
        if client.get(format!("{origin}/api/uploads/{writing}")).bearer_auth(&token).send().await.unwrap().status()==202 {break;}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.unwrap();
    let cancelling=client.delete(format!("{origin}/api/uploads/{writing}")).bearer_auth(&token).send().await.unwrap();assert_eq!(cancelling.status(),202);
    let state:Value=cancelling.json().await.unwrap();assert_eq!(state["cancelled"],false);
    let mut stopped=Vec::new();tokio::time::timeout(Duration::from_secs(2),chunk.read_to_end(&mut stopped)).await.unwrap().unwrap();assert!(String::from_utf8_lossy(&stopped).starts_with("HTTP/1.1 409"));
    tokio::time::timeout(Duration::from_secs(2),async {loop {
        let response=client.get(format!("{origin}/api/uploads/{writing}")).bearer_auth(&token).send().await.unwrap();let code=response.status();let status:Value=response.json().await.unwrap();
        if code==200 {assert_eq!(status["cancelled"],true);break;}assert_eq!(code,202);tokio::task::yield_now().await;
    }}).await.unwrap();
    assert!(!root.path().join("cancelled.bin").exists());
    let published="7".repeat(32);let done=upload_resumable(&client,&origin,&token,&published,"saved.bin",b"saved").await;assert_eq!(done.status(),200);
    tokio::time::timeout(Duration::from_secs(2),async {loop {
        let response=client.delete(format!("{origin}/api/uploads/{published}")).bearer_auth(&token).send().await.unwrap();let code=response.status();let status:Value=response.json().await.unwrap();
        if code==200 {assert_eq!(status["complete"],true);assert_eq!(status["cancelled"],false);assert_eq!(status["name"],"saved.bin");break;}
        assert_eq!(code,202);tokio::task::yield_now().await;
    }}).await.unwrap();
    let again:Value=client.post(format!("{origin}/api/uploads/{published}")).bearer_auth(&token).send().await.unwrap().json().await.unwrap();assert_eq!(again["complete"],true);
    assert_eq!(std::fs::read(root.path().join("saved.bin")).unwrap(),b"saved");assert_eq!(std::fs::read_dir(root.path()).unwrap().count(),1);
    server.stop().await;controller.shutdown();
}

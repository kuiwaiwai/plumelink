use axum::{extract::{Request, State}, http::{HeaderMap, Method, StatusCode, Uri}, middleware::Next, response::Response};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use super::{Context, error};

pub(crate) const EXPIRED: &str = "USB 连接已失效，请在电脑「偏好设置 → USB 有线连接」中重新连接手机";

#[derive(Clone)]
pub(super) struct UsbAccess { key: String, pub tag: String }
impl UsbAccess {
    pub fn new()->Result<Self,String> {
        let mut bytes=[0;32];getrandom::fill(&mut bytes).map_err(|_|"无法生成 USB 连接凭据，请重试")?;
        let key=hex::encode(bytes);let tag=hex::encode(Sha256::digest(key.as_bytes()));Ok(Self{key,tag})
    }
    pub fn key(&self)->&str {&self.key}
    fn matches(&self,value:&str)->bool {self.key.as_bytes().ct_eq(value.as_bytes()).into()}
}
impl Context {
    pub(super) fn usb_allowed(&self,headers:&HeaderMap,uri:&Uri,websocket:bool)->bool {
        let Some(access)=&self.usb_access else {return true;};
        let mut headers=headers.get_all("x-plumelink-usb").iter();
        let header=match headers.next() {Some(value)=>match value.to_str(){Ok(value)=>Some(value),Err(_)=>return false},None=>None};
        if headers.next().is_some(){return false;}
        let mut query=uri.query().unwrap_or("").split('&').map(|part|part.split_once('=').unwrap_or((part,""))).filter(|(key,_)|*key=="usb");
        let query_key=query.next().map(|(_,value)|value);
        if query.next().is_some()||query_key.is_some()&&(!websocket||header.is_some()){return false;}
        header.or(query_key).is_some_and(|value|access.matches(value))
    }
}
pub(super) async fn guard(State(context):State<Arc<Context>>,request:Request,next:Next)->Response {
    let public_identity=request.method()==Method::GET&&request.uri().path()=="/api/identity";
    let input_socket=request.method()==Method::GET&&request.uri().path()=="/ws";
    if !public_identity&&!input_socket&&!context.usb_allowed(request.headers(),request.uri(),false) {return error(StatusCode::GONE,EXPIRED);}
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{control::{Controller,Native},file_transfer::FileHub,preferences::Store,protocol::Command,server::{Server,ServingEndpoint}};
    use futures_util::{SinkExt,StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest,Message};
    struct NoInput;
    impl Native for NoInput {fn execute(&mut self,_command:&Command)->Result<(),String>{Err("test input disabled".into())}fn release(&mut self)->Result<(),String>{Ok(())}}
    #[tokio::test]
    async fn a_reused_usb_port_requires_a_fresh_capability_but_preserves_pc_pairing() {
        let controller=Controller::start(||Ok(Box::new(NoInput))).unwrap();let store=Store::memory();
        let mut server=Server::start_with_files(std::net::Ipv4Addr::LOCALHOST,0,controller.clone(),store.clone(),FileHub::new()).await.unwrap();
        let address=server.ensure_usb_for("first").await.unwrap();let origin=format!("http://{address}");let old=server.usb_access_for("first").unwrap().to_string();
        let client=reqwest::Client::builder().no_proxy().build().unwrap();let code=server.context.code();
        let identity:serde_json::Value=client.get(format!("{origin}/api/identity")).send().await.unwrap().json().await.unwrap();assert_eq!(identity["usbSession"],hex::encode(Sha256::digest(old.as_bytes())));assert!(!identity.to_string().contains(&old));
        let paired:serde_json::Value=client.post(format!("{origin}/api/pair")).header("x-plumelink-usb",&old).json(&serde_json::json!({"code":code})).send().await.unwrap().json().await.unwrap();let token=paired["token"].as_str().unwrap();
        server.stop_usb_for("first").await;
        let socket=tokio::net::TcpSocket::new_v4().unwrap();socket.set_reuseaddr(true).unwrap();socket.bind(address).unwrap();let listener=socket.listen(128).unwrap();
        let endpoint=ServingEndpoint::start(listener,None,&server.context,false,Some(UsbAccess::new().unwrap()));server.usb.insert("next".into(),endpoint);
        let key=server.usb_access_for("next").unwrap();assert_ne!(key,old);
        for path in ["/api/status","/api/windows","/api/files","/api/uploads","/microphone"] {
            assert_eq!(client.get(format!("{origin}{path}")).bearer_auth(token).header("x-plumelink-usb",&old).send().await.unwrap().status(),410);
            assert_eq!(client.get(format!("{origin}{path}")).bearer_auth(token).send().await.unwrap().status(),410);
        }
        assert_eq!(client.post(format!("{origin}/api/pair")).json(&serde_json::json!({"code":code})).send().await.unwrap().status(),410);assert_eq!(store.lock().unwrap().get().devices.len(),1);
        assert_eq!(client.get(format!("{origin}/api/status")).bearer_auth(token).header("x-plumelink-usb",key).send().await.unwrap().status(),200);
        assert_eq!(client.get(format!("{}/api/status",server.context.origin)).bearer_auth(token).send().await.unwrap().status(),200);
        for query in [String::new(),format!("?usb={old}"),format!("?usb={key}&usb={key}"),format!("?usb&usb={key}")] {
            let mut request=format!("ws://{address}/ws{query}").into_client_request().unwrap();request.headers_mut().insert("origin",origin.parse().unwrap());
            let mut ws=tokio_tungstenite::connect_async(request).await.unwrap().0;
            let frame=ws.next().await.unwrap().unwrap();assert!(matches!(frame,Message::Close(Some(frame)) if frame.code==4409u16.into()));assert_eq!(server.context.connected_count(),0);
        }
        let mut request=format!("ws://{address}/ws?usb={key}").into_client_request().unwrap();request.headers_mut().insert("origin",origin.parse().unwrap());
        let mut ws=tokio_tungstenite::connect_async(request).await.unwrap().0;ws.send(Message::Text(serde_json::json!({"kind":"auth","token":token}).to_string().into())).await.unwrap();assert!(ws.next().await.unwrap().unwrap().into_text().unwrap().contains("ready"));
        server.stop().await;controller.shutdown();
    }
    #[tokio::test]
    async fn endpoint_rotation_and_server_recreation_never_reuse_the_usb_capability() {
        let controller=Controller::start(||Ok(Box::new(NoInput))).unwrap();let store=Store::memory();let files=FileHub::new();let mut keys=std::collections::HashSet::new();
        for _ in 0..3 {let mut server=Server::start_usb_only(controller.clone(),store.clone(),files.clone()).await.unwrap();server.ensure_usb_for("phone").await.unwrap();assert!(keys.insert(server.usb_access_for("phone").unwrap().to_string()));server.rotate_usb_for("phone").await.unwrap();assert!(keys.insert(server.usb_access_for("phone").unwrap().to_string()));server.stop().await;}
        controller.shutdown();
    }
}

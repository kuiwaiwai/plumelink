use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tauri::{
    ipc::{CallbackFn, InvokeBody},
    test::{get_ipc_response, mock_builder, MockRuntime, INVOKE_KEY},
    webview::InvokeRequest,
    WebviewWindow,
};

fn request(
    window: &WebviewWindow<MockRuntime>,
    command: &str,
    origin: &str,
) -> Result<String, serde_json::Value> {
    get_ipc_response(
        window,
        InvokeRequest {
            cmd: command.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: origin.parse().unwrap(),
            body: InvokeBody::default(),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.into(),
        },
    )
    .map(|response| response.deserialize().unwrap())
}

#[test]
fn compiled_acl_allows_desktop_commands_only_from_the_local_main_window() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let app = mock_builder()
        .invoke_handler(move |invoke| {
            observed.fetch_add(1, Ordering::SeqCst);
            invoke.resolver.resolve("handler reached");
            true
        })
        .build(tauri::generate_context!())
        .unwrap();
    let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    let other = tauri::WebviewWindowBuilder::new(&app, "other", Default::default())
        .build()
        .unwrap();
    let capability: serde_json::Value =
        serde_json::from_str(include_str!("../capabilities/desktop.json")).unwrap();
    let commands: Vec<_> = capability["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry.as_str())
        .filter(|entry| !entry.contains(':'))
        .map(|entry| entry.strip_prefix("allow-").unwrap().replace('-', "_"))
        .collect();
    assert!(commands.iter().any(|command| command == "usb_action"));
    assert!(commands.iter().any(|command| command == "receive_action"));
    for command in &commands {
        assert_eq!(
            request(&main, command, "http://tauri.localhost").unwrap(),
            "handler reached"
        );
        assert!(
            request(&other, command, "http://tauri.localhost").is_err(),
            "unexpected window allowed {command}"
        );
        assert!(
            request(&main, command, "https://example.invalid").is_err(),
            "remote page allowed {command}"
        );
    }
    assert!(request(&main, "unregistered_command", "http://tauri.localhost").is_err());
    assert_eq!(calls.load(Ordering::SeqCst), commands.len());
}

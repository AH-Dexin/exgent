//! End-to-end integration tests that exercise `AppRuntimeHost` from the
//! consumer side, with the `fake` provider for deterministic behavior.

use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use exgent_ai::ToolCall;
use exgent_core::{
    AgentEvent, AgentHooks, AgentLoopConfig, AgentSessionEvent, AppRuntimeHost, CancelToken,
    RuntimeOptions, ToolExecutionResult, TurnTelemetry,
};

fn fixture(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("exgent_integration_{name}_{stamp}"))
}

fn write_fake_config(dir: &PathBuf) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("models.json"),
        r#"{
          "models": [
            {
              "provider": "test-provider",
              "id": "test-model",
              "adapter": "fake"
            }
          ]
        }"#,
    )
    .unwrap();
    fs::write(
        dir.join("settings.json"),
        r#"{"default_model":{"provider":"test-provider","id":"test-model","adapter":"fake"}}"#,
    )
    .unwrap();
}

fn collect_events(
    host: &mut AppRuntimeHost,
    prompt: &str,
    cancel: &CancelToken,
) -> Vec<AgentEvent> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let _ = host.run_prompt_events_cancellable(prompt, cancel, &mut |event| {
        if let AgentSessionEvent::Agent(event) = event {
            sink.lock().unwrap().push(event);
        }
    });
    let events = events.lock().unwrap().clone();
    events
}

#[test]
fn host_runs_prompt_end_to_end_with_fake_provider() {
    let dir = fixture("host_runs_prompt_end_to_end_with_fake_provider");
    write_fake_config(&dir);

    let mut host = AppRuntimeHost::new(RuntimeOptions {
        config_path: Some(dir.display().to_string()),
        enable_dev_providers: true,
        ..RuntimeOptions::default()
    })
    .unwrap();

    let events = collect_events(&mut host, "hello world", &CancelToken::new());
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::MessageEnd { content } if content.starts_with("fake response:")
    )));
    assert_eq!(events.last(), Some(&AgentEvent::AgentEnd));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn host_honors_cancellation_between_rounds() {
    let dir = fixture("host_honors_cancellation_between_rounds");
    write_fake_config(&dir);

    let mut host = AppRuntimeHost::new(RuntimeOptions {
        config_path: Some(dir.display().to_string()),
        agent: AgentLoopConfig { max_tool_rounds: 8 },
        enable_dev_providers: true,
    })
    .unwrap();

    let cancel = CancelToken::new();
    cancel.cancel();
    let events = collect_events(&mut host, "tool read Cargo.toml", &cancel);

    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::Error { message } if message == "cancelled"
    )));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn host_emits_turn_telemetry_event() {
    let dir = fixture("host_emits_turn_telemetry_event");
    write_fake_config(&dir);

    let mut host = AppRuntimeHost::new(RuntimeOptions {
        config_path: Some(dir.display().to_string()),
        enable_dev_providers: true,
        ..RuntimeOptions::default()
    })
    .unwrap();

    let telemetry: Arc<Mutex<Option<TurnTelemetry>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&telemetry);
    host.run_prompt_events_cancellable("hi", &CancelToken::new(), &mut |event| {
        if let AgentSessionEvent::TurnTelemetry(value) = event {
            *sink.lock().unwrap() = Some(value);
        }
    })
    .unwrap();

    let recorded = telemetry
        .lock()
        .unwrap()
        .clone()
        .expect("telemetry emitted");
    assert!(!recorded.errored);
    assert_eq!(recorded.tool_calls, 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn host_hooks_can_short_circuit_tools() {
    let dir = fixture("host_hooks_can_short_circuit_tools");
    write_fake_config(&dir);

    let mut host = AppRuntimeHost::new(RuntimeOptions {
        config_path: Some(dir.display().to_string()),
        enable_dev_providers: true,
        ..RuntimeOptions::default()
    })
    .unwrap();

    struct Block;
    impl AgentHooks for Block {
        fn before_tool_call(&self, call: &ToolCall) -> Option<ToolExecutionResult> {
            Some(ToolExecutionResult {
                content: format!("denied: {}", call.name),
                is_error: true,
            })
        }
    }

    host.set_hooks(Arc::new(Block));

    let events = collect_events(&mut host, "tool read Cargo.toml", &CancelToken::new());
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ToolCallEnd { content, is_error: true, .. } if content == "denied: read"
    )));

    let _ = fs::remove_dir_all(&dir);
}

//! Integration tests for headless mode

use kn_code_server::headless::events::SdkEvent;
use serde_json;

#[tokio::test]
async fn test_jsonl_event_serialization() {
    let event = SdkEvent::session_init("test-session", "anthropic/claude-sonnet-4-5");
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("session_init"));
    assert!(json.contains("test-session"));
}

#[tokio::test]
async fn test_result_success_serialization() {
    let event = SdkEvent::result_success(
        "session-123",
        kn_code_server::headless::events::TokenUsage::default(),
        0.05,
        "Task completed".to_string(),
    );
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("result"));
    assert!(json.contains("success"));
    assert!(json.contains("session-123"));
}

#[tokio::test]
async fn test_error_serialization() {
    let event = SdkEvent::error("Something went wrong", Some("TEST_ERROR"));
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("error"));
    assert!(json.contains("Something went wrong"));
}

#[tokio::test]
async fn test_unknown_session_error_format() {
    let event = SdkEvent::unknown_session("abc-123");
    let json = serde_json::to_string(&event).unwrap();
    // Paperclip regex: /unknown\s+session/i
    assert!(json.contains("unknown session"));
}

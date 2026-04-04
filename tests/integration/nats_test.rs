//! Integration tests for NATS pubsub

use kn_code_nats::{NatsConfig, NatsConnection, Publisher, SubscriptionManager};

#[tokio::test]
async fn test_nats_config_from_env() {
    let config = NatsConfig::from_env();
    assert!(!config.url.is_empty());
}

#[tokio::test]
async fn test_nats_config_instance_id() {
    let config = NatsConfig::default();
    let id = config.instance_id();
    assert!(!id.is_empty());
    assert_eq!(id.len(), 8);
}

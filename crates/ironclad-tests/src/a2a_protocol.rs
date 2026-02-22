use ironclad_channels::a2a::A2aProtocol;
use ironclad_core::config::A2aConfig;

#[test]
fn a2a_hello_handshake() {
    let config = A2aConfig::default();
    let _proto = A2aProtocol::new(config);
    let nonce = [0u8; 12];
    let hello = A2aProtocol::generate_hello("did:test:alice", &nonce);
    assert_eq!(hello["type"], "a2a_hello");
    assert_eq!(hello["did"], "did:test:alice");
    let result = A2aProtocol::verify_hello(&hello);
    assert!(result.is_ok());
}

#[test]
fn a2a_message_size_limit() {
    let mut config = A2aConfig::default();
    config.max_message_size = 100;
    let proto = A2aProtocol::new(config);
    let big = vec![0u8; 200];
    let result = proto.validate_message_size(&big);
    assert!(result.is_err());
}

#[test]
fn a2a_timestamp_validation() {
    let ts = chrono::Utc::now().timestamp();
    assert!(A2aProtocol::validate_timestamp(ts, 300).is_ok());
    let old_ts = ts - 600;
    assert!(A2aProtocol::validate_timestamp(old_ts, 300).is_err());
}

#[test]
fn a2a_ecdh_handshake_encrypted_roundtrip() {
    let (secret_a, pub_a) = A2aProtocol::generate_keypair();
    let (secret_b, pub_b) = A2aProtocol::generate_keypair();

    let key_a = A2aProtocol::derive_session_key(secret_a, &pub_b);
    let key_b = A2aProtocol::derive_session_key(secret_b, &pub_a);
    assert_eq!(key_a, key_b, "both sides should derive the same session key");

    let plaintext = b"hello agent-to-agent";
    let ciphertext = A2aProtocol::encrypt_message(&key_a, plaintext).unwrap();
    let decrypted = A2aProtocol::decrypt_message(&key_b, &ciphertext).unwrap();
    assert_eq!(decrypted, plaintext);
}

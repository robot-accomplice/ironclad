use chrono::{DateTime, Utc};
use ironclad_core::{IroncladError, Result};
use k256::ecdsa::SigningKey;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use tracing::{debug, info};

/// Unique device identity derived from an ECDSA keypair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub public_key_hex: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub device_name: String,
}

impl DeviceIdentity {
    /// Generate a new device identity with a random ID.
    pub fn generate(device_name: &str) -> Self {
        let device_id = format!("dev_{}", generate_short_id());
        let public_key_hex = generate_pubkey();

        info!(device_id = %device_id, name = %device_name, "generated device identity");

        Self {
            device_id,
            public_key_hex,
            created_at: Utc::now(),
            device_name: device_name.to_string(),
        }
    }

    pub fn fingerprint(&self) -> String {
        let hash = sha2::Sha256::digest(self.public_key_hex.as_bytes());
        hex::encode(&hash[..8])
    }
}

/// Pairing state for device-to-device trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairingState {
    Pending,
    Verified,
    Rejected,
    Expired,
}

/// A paired device record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    pub device_id: String,
    pub public_key_hex: String,
    pub device_name: String,
    pub state: PairingState,
    pub paired_at: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
}

/// Manages device identity and pairing.
pub struct DeviceManager {
    identity: DeviceIdentity,
    paired_devices: HashMap<String, PairedDevice>,
    max_paired: usize,
}

impl DeviceManager {
    pub fn new(identity: DeviceIdentity, max_paired: usize) -> Self {
        Self {
            identity,
            paired_devices: HashMap::new(),
            max_paired,
        }
    }

    pub fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    /// Initiate pairing with another device.
    pub fn initiate_pairing(
        &mut self,
        remote_id: &str,
        remote_pubkey: &str,
        remote_name: &str,
    ) -> Result<()> {
        if self.paired_devices.len() >= self.max_paired {
            return Err(IroncladError::Config(format!(
                "maximum paired devices ({}) reached",
                self.max_paired
            )));
        }

        if self.paired_devices.contains_key(remote_id) {
            return Err(IroncladError::Config(format!(
                "device '{}' is already in pairing list",
                remote_id
            )));
        }

        self.paired_devices.insert(
            remote_id.to_string(),
            PairedDevice {
                device_id: remote_id.to_string(),
                public_key_hex: remote_pubkey.to_string(),
                device_name: remote_name.to_string(),
                state: PairingState::Pending,
                paired_at: None,
                last_seen: None,
            },
        );

        debug!(remote = %remote_id, "pairing initiated");
        Ok(())
    }

    /// Verify a pending pairing (after mutual authentication succeeds).
    pub fn verify_pairing(&mut self, remote_id: &str) -> Result<()> {
        let device = self
            .paired_devices
            .get_mut(remote_id)
            .ok_or_else(|| IroncladError::Config(format!("device '{}' not found", remote_id)))?;

        if device.state != PairingState::Pending {
            return Err(IroncladError::Config(format!(
                "device '{}' is not in pending state",
                remote_id
            )));
        }

        device.state = PairingState::Verified;
        device.paired_at = Some(Utc::now());
        device.last_seen = Some(Utc::now());

        info!(remote = %remote_id, "pairing verified");
        Ok(())
    }

    /// Reject a pending pairing.
    pub fn reject_pairing(&mut self, remote_id: &str) -> Result<()> {
        let device = self
            .paired_devices
            .get_mut(remote_id)
            .ok_or_else(|| IroncladError::Config(format!("device '{}' not found", remote_id)))?;

        device.state = PairingState::Rejected;
        debug!(remote = %remote_id, "pairing rejected");
        Ok(())
    }

    /// Remove a device from the pairing list.
    pub fn unpair(&mut self, remote_id: &str) -> Result<()> {
        self.paired_devices
            .remove(remote_id)
            .ok_or_else(|| IroncladError::Config(format!("device '{}' not found", remote_id)))?;

        info!(remote = %remote_id, "device unpaired");
        Ok(())
    }

    /// Record that a paired device was seen (for sync/heartbeat).
    pub fn record_seen(&mut self, remote_id: &str) {
        if let Some(device) = self.paired_devices.get_mut(remote_id) {
            device.last_seen = Some(Utc::now());
        }
    }

    /// List all verified (trusted) devices.
    pub fn trusted_devices(&self) -> Vec<&PairedDevice> {
        self.paired_devices
            .values()
            .filter(|d| d.state == PairingState::Verified)
            .collect()
    }

    /// List all paired devices regardless of state.
    pub fn all_devices(&self) -> Vec<&PairedDevice> {
        self.paired_devices.values().collect()
    }

    pub fn paired_count(&self) -> usize {
        self.paired_devices.len()
    }

    /// Check if a device is trusted (verified pairing).
    pub fn is_trusted(&self, remote_id: &str) -> bool {
        self.paired_devices
            .get(remote_id)
            .is_some_and(|d| d.state == PairingState::Verified)
    }
}

fn generate_short_id() -> String {
    format!("{:08x}", rand::random::<u32>())
}

fn generate_pubkey() -> String {
    let signing_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
    let point = signing_key.verifying_key().to_encoded_point(true);
    hex::encode(point.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity() -> DeviceIdentity {
        DeviceIdentity::generate("test-device")
    }

    fn test_manager() -> DeviceManager {
        DeviceManager::new(test_identity(), 5)
    }

    #[test]
    fn generate_identity() {
        let id = DeviceIdentity::generate("laptop");
        assert!(id.device_id.starts_with("dev_"));
        assert!(!id.public_key_hex.is_empty());
        assert_eq!(id.device_name, "laptop");
    }

    #[test]
    fn identity_fingerprint() {
        let id = test_identity();
        let fp = id.fingerprint();
        assert_eq!(fp.len(), 16);
    }

    #[test]
    fn initiate_pairing() {
        let mut mgr = test_manager();
        mgr.initiate_pairing("remote-1", "04abcdef", "phone")
            .unwrap();
        assert_eq!(mgr.paired_count(), 1);
        assert!(!mgr.is_trusted("remote-1"));
    }

    #[test]
    fn verify_pairing() {
        let mut mgr = test_manager();
        mgr.initiate_pairing("remote-1", "04abcdef", "phone")
            .unwrap();
        mgr.verify_pairing("remote-1").unwrap();
        assert!(mgr.is_trusted("remote-1"));
        assert_eq!(mgr.trusted_devices().len(), 1);
    }

    #[test]
    fn reject_pairing() {
        let mut mgr = test_manager();
        mgr.initiate_pairing("remote-1", "04abcdef", "phone")
            .unwrap();
        mgr.reject_pairing("remote-1").unwrap();
        assert!(!mgr.is_trusted("remote-1"));
    }

    #[test]
    fn unpair() {
        let mut mgr = test_manager();
        mgr.initiate_pairing("remote-1", "04abcdef", "phone")
            .unwrap();
        mgr.unpair("remote-1").unwrap();
        assert_eq!(mgr.paired_count(), 0);
    }

    #[test]
    fn max_paired_limit() {
        let mut mgr = DeviceManager::new(test_identity(), 2);
        mgr.initiate_pairing("d1", "key1", "dev1").unwrap();
        mgr.initiate_pairing("d2", "key2", "dev2").unwrap();
        let err = mgr.initiate_pairing("d3", "key3", "dev3").unwrap_err();
        assert!(err.to_string().contains("maximum"));
    }

    #[test]
    fn duplicate_pairing_rejected() {
        let mut mgr = test_manager();
        mgr.initiate_pairing("d1", "key1", "dev1").unwrap();
        let err = mgr.initiate_pairing("d1", "key1", "dev1").unwrap_err();
        assert!(err.to_string().contains("already"));
    }

    #[test]
    fn verify_nonexistent_fails() {
        let mut mgr = test_manager();
        assert!(mgr.verify_pairing("nope").is_err());
    }

    #[test]
    fn verify_non_pending_fails() {
        let mut mgr = test_manager();
        mgr.initiate_pairing("d1", "key1", "dev1").unwrap();
        mgr.verify_pairing("d1").unwrap();
        assert!(mgr.verify_pairing("d1").is_err());
    }

    #[test]
    fn record_seen() {
        let mut mgr = test_manager();
        mgr.initiate_pairing("d1", "key1", "dev1").unwrap();
        mgr.verify_pairing("d1").unwrap();
        mgr.record_seen("d1");
        let devs = mgr.trusted_devices();
        assert!(devs[0].last_seen.is_some());
    }

    #[test]
    fn pairing_state_serde() {
        for state in [
            PairingState::Pending,
            PairingState::Verified,
            PairingState::Rejected,
            PairingState::Expired,
        ] {
            let json = serde_json::to_string(&state).unwrap();
            let back: PairingState = serde_json::from_str(&json).unwrap();
            assert_eq!(state, back);
        }
    }

    #[test]
    fn identity_serde() {
        let id = test_identity();
        let json = serde_json::to_string(&id).unwrap();
        let back: DeviceIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(id.device_id, back.device_id);
    }
}

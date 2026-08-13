//! Small transport vocabulary shared by the semantic contract and adapters.
//!
//! The current executor still uses the mature ADB implementation underneath.
//! These types make the boundary explicit without pretending remote mode is
//! raw ADB or introducing an inheritance-heavy device framework.

use serde::{Deserialize, Serialize};

use crate::device::{Endpoint, EndpointKind};
use crate::error::{AuError, Result};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Usb,
    LocalWifi,
    LocalMdns,
    Remote,
    Emulator,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransportDescriptor {
    pub kind: TransportKind,
    pub endpoint: Option<String>,
    pub identity: Option<String>,
    pub latency_class: String,
    pub raw_adb: bool,
}

impl TransportDescriptor {
    pub fn from_endpoint(endpoint: &Endpoint) -> Self {
        let kind = match endpoint.kind {
            EndpointKind::Usb => TransportKind::Usb,
            EndpointKind::Wifi => TransportKind::LocalWifi,
            EndpointKind::Mdns => TransportKind::LocalMdns,
            EndpointKind::Other => TransportKind::LocalWifi,
        };
        let latency_class = match kind {
            TransportKind::Usb => "low",
            TransportKind::LocalWifi | TransportKind::LocalMdns => "variable",
            TransportKind::Remote => "wan",
            TransportKind::Emulator => "local",
        };
        Self {
            kind,
            endpoint: Some(endpoint.endpoint.clone()),
            identity: endpoint.hardware_serial.clone(),
            latency_class: latency_class.into(),
            raw_adb: true,
        }
    }

    pub fn remote(identity: String) -> Self {
        Self {
            kind: TransportKind::Remote,
            endpoint: None,
            identity: Some(identity),
            latency_class: "wan".into(),
            raw_adb: false,
        }
    }

    pub fn validate_semantic_only(&self) -> Result<()> {
        if self.kind == TransportKind::Remote && self.raw_adb {
            return Err(AuError::code(
                "E_REMOTE_POLICY",
                "remote transport cannot expose raw ADB",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_transport_is_not_raw_adb() {
        let transport = TransportDescriptor::remote("device-1".into());
        assert!(!transport.raw_adb);
        transport.validate_semantic_only().expect("safe remote");
    }
}

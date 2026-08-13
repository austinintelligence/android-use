use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::json;

use crate::adb::Adb;
use crate::config::Config;
use crate::error::{AuError, Result};
use crate::process::text;
use crate::trace;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EndpointKind {
    Usb,
    Wifi,
    Mdns,
    Other,
}

#[derive(Clone, Debug, Serialize)]
pub struct Endpoint {
    pub endpoint: String,
    pub state: String,
    pub kind: EndpointKind,
    pub product: Option<String>,
    pub model: Option<String>,
    pub transport_id: Option<String>,
    pub hardware_serial: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceInventory {
    pub endpoints: Vec<Endpoint>,
}

/// Daemon-lifetime selection cache.  The cache stores only a fully resolved
/// endpoint whose hardware serial was checked against the canonical config.
/// Callers must invalidate it after transport/config changes or a device
/// transport error; it is never used to authorize a different serial.
#[derive(Clone, Debug, Default)]
pub struct SelectionCache {
    endpoint: Option<Endpoint>,
}

impl SelectionCache {
    pub fn resolve(
        &mut self,
        adb: &Adb,
        config: &Config,
        requested: Option<&str>,
    ) -> Result<Endpoint> {
        if let Some(endpoint) = self.endpoint.as_ref() {
            if config.identity_matches(endpoint.hardware_serial.as_deref())
                && endpoint_matches_requested(endpoint, requested)
            {
                // A cached endpoint is usable only while the same live ADB
                // transport is present. USB serials are stable device
                // identity, but a reconnect can reuse the endpoint with a
                // different transport (and a daemon must not carry stale
                // helper/forward state across that boundary).
                if endpoint.transport_id.is_some() {
                    let current = DeviceInventory::discover_endpoints(adb)?;
                    if current
                        .iter()
                        .any(|candidate| same_transport_connection(endpoint, candidate))
                    {
                        return Ok(endpoint.clone());
                    }
                }
            }
        }
        let inventory = DeviceInventory::discover_for_identity(
            adb,
            config.enrolled_serial().unwrap_or_default(),
        )?;
        let endpoint = inventory.resolve(config, requested)?;
        self.endpoint = Some(endpoint.clone());
        Ok(endpoint)
    }

    pub fn invalidate(&mut self) {
        self.endpoint = None;
    }
}

/// Decide whether a cached endpoint still satisfies the caller's request.
/// Symbolic selectors such as `USB` and `MDNS` must match by endpoint kind;
/// literal endpoint names must match by exact value. Keeping this beside the
/// resolver prevents the daemon cache from rediscovering the device on every
/// request merely because the caller used a symbolic selector.
pub(crate) fn endpoint_matches_requested(endpoint: &Endpoint, requested: Option<&str>) -> bool {
    requested.is_none_or(|requested| {
        endpoint_kind_selector(requested).is_some_and(|kind| kind == endpoint.kind)
            || requested == endpoint.endpoint
    })
}

impl DeviceInventory {
    pub fn discover(adb: &Adb) -> Result<Self> {
        Self::discover_with_identity(adb, None)
    }

    /// Discover endpoints and avoid a redundant on-device identity probe when
    /// ADB's USB serial is already the canonical configured hardware serial.
    /// Wi-Fi and mDNS transports are always probed because their endpoint name
    /// is not itself an identity proof.
    pub fn discover_for_identity(adb: &Adb, hardware_serial: &str) -> Result<Self> {
        Self::discover_with_identity(
            adb,
            (!hardware_serial.is_empty()).then_some(hardware_serial),
        )
    }

    fn discover_with_identity(adb: &Adb, configured_serial: Option<&str>) -> Result<Self> {
        let _span = trace::span(
            "device.discover",
            json!({"configured":configured_serial.is_some()}),
        );
        let mut endpoints = Self::discover_endpoints(adb)?;
        for endpoint in &mut endpoints {
            if endpoint.state != "device" {
                continue;
            }
            if endpoint.kind == EndpointKind::Usb
                && configured_serial.is_some_and(|serial| serial == endpoint.endpoint)
            {
                endpoint.hardware_serial = Some(endpoint.endpoint.clone());
                continue;
            }
            let response = adb.device(
                &endpoint.endpoint,
                &["shell".into(), "getprop".into(), "ro.serialno".into()],
            );
            if let Ok(response) = response {
                let value = text(&response.stdout);
                if !value.is_empty() {
                    endpoint.hardware_serial = Some(value);
                }
            }
        }
        Ok(Self { endpoints })
    }

    fn discover_endpoints(adb: &Adb) -> Result<Vec<Endpoint>> {
        Ok(parse_devices(&adb.devices_long()?))
    }

    pub fn resolve(&self, config: &Config, requested: Option<&str>) -> Result<Endpoint> {
        let _span = trace::span(
            "device.resolve",
            json!({"n":self.endpoints.len(),"requested":requested}),
        );
        let matches_identity = |endpoint: &&Endpoint| {
            endpoint.state == "device"
                && (config.enrolled_serial().is_some()
                    && config.identity_matches(endpoint.hardware_serial.as_deref()))
        };
        if let Some(requested) = requested {
            if let Some(kind) = endpoint_kind_selector(requested) {
                let matches = self
                    .endpoints
                    .iter()
                    .filter(|endpoint| endpoint.kind == kind && endpoint.state == "device")
                    .filter(|endpoint| {
                        config.enrolled_serial().is_none()
                            || config.identity_matches(endpoint.hardware_serial.as_deref())
                    })
                    .collect::<Vec<_>>();
                return match matches.as_slice() {
                    [endpoint] => Ok((*endpoint).clone()),
                    [] if config.enrolled_serial().is_none() => Err(AuError::code(
                        "E_ENROLL",
                        format!(
                            "no unique online {requested} endpoint; run au d and enroll an exact endpoint with au u ENDPOINT"
                        ),
                    )),
                    [] => Err(AuError::code(
                        "E_DEVICE",
                        format!("no online {requested} endpoint matches the enrolled hardware"),
                    )),
                    _ => Err(AuError::code(
                        "E_DEVICE",
                        format!("multiple online {requested} endpoints; use au u ENDPOINT"),
                    )),
                };
            }
            let endpoint = self
                .endpoints
                .iter()
                .find(|endpoint| endpoint.endpoint == requested)
                .ok_or_else(|| {
                    AuError::code("E_DEVICE", format!("endpoint {requested} is not connected"))
                })?;
            if endpoint.state != "device" {
                return Err(AuError::code(
                    "E_DEVICE",
                    format!("endpoint {requested} is not authorized"),
                ));
            }
            if config.enrolled_serial().is_some() && !matches_identity(&endpoint) {
                return Err(AuError::code(
                    "E_IDENTITY",
                    format!("endpoint {requested} does not match the enrolled hardware"),
                ));
            }
            if endpoint.hardware_serial.is_none() {
                return Err(AuError::code(
                    "E_IDENTITY",
                    format!("endpoint {requested} did not report ro.serialno"),
                ));
            }
            return Ok(endpoint.clone());
        }
        if config.enrolled_serial().is_none() {
            return Err(AuError::code(
                "E_ENROLL",
                "no Android device is enrolled; run au d, then au u ENDPOINT",
            ));
        }
        if let Some(endpoint) = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.kind == EndpointKind::Usb && matches_identity(endpoint))
        {
            return Ok(endpoint.clone());
        }
        if let Some(endpoint) = self.endpoints.iter().find(|endpoint| {
            endpoint.kind == EndpointKind::Wifi
                && config
                    .known_wifi_endpoints
                    .iter()
                    .any(|known| known == &endpoint.endpoint)
                && matches_identity(endpoint)
        }) {
            return Ok(endpoint.clone());
        }
        if let Some(endpoint) = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.kind == EndpointKind::Mdns && matches_identity(endpoint))
        {
            return Ok(endpoint.clone());
        }
        Err(AuError::code(
            "E_DEVICE",
            "no online endpoint matches the enrolled hardware",
        ))
    }

    pub fn identities(&self) -> BTreeMap<String, Vec<String>> {
        let mut groups = BTreeMap::new();
        for endpoint in &self.endpoints {
            if let Some(identity) = &endpoint.hardware_serial {
                groups
                    .entry(identity.clone())
                    .or_insert_with(Vec::new)
                    .push(endpoint.endpoint.clone());
            }
        }
        groups
    }
}

fn same_transport_connection(cached: &Endpoint, current: &Endpoint) -> bool {
    cached.endpoint == current.endpoint
        && current.state == "device"
        && cached.transport_id.is_some()
        && cached.transport_id == current.transport_id
}

pub(crate) fn endpoint_kind_selector(value: &str) -> Option<EndpointKind> {
    match value.to_ascii_lowercase().as_str() {
        "usb" => Some(EndpointKind::Usb),
        "wifi" | "wireless" => Some(EndpointKind::Wifi),
        "mdns" => Some(EndpointKind::Mdns),
        _ => None,
    }
}

pub fn parse_devices(input: &str) -> Vec<Endpoint> {
    input
        .lines()
        .skip_while(|line| !line.starts_with("List of devices attached"))
        .skip(1)
        .filter_map(parse_device_line)
        .collect()
}

fn parse_device_line(line: &str) -> Option<Endpoint> {
    let mut fields = line.split_whitespace();
    let endpoint = fields.next()?.to_owned();
    let state = fields.next()?.to_owned();
    let mut product = None;
    let mut model = None;
    let mut transport_id = None;
    for field in fields {
        if let Some(value) = field.strip_prefix("product:") {
            product = Some(value.into());
        } else if let Some(value) = field.strip_prefix("model:") {
            model = Some(value.into());
        } else if let Some(value) = field.strip_prefix("transport_id:") {
            transport_id = Some(value.into());
        }
    }
    let kind = if endpoint.ends_with("_adb-tls-connect._tcp") {
        EndpointKind::Mdns
    } else if endpoint.contains(':') {
        EndpointKind::Wifi
    } else if endpoint
        .chars()
        .all(|character| character.is_ascii_hexdigit())
    {
        EndpointKind::Usb
    } else {
        EndpointKind::Other
    };
    Some(Endpoint {
        endpoint,
        state,
        kind,
        product,
        model,
        transport_id,
        hardware_serial: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_devices, same_transport_connection, DeviceInventory, EndpointKind};
    use crate::config::Config;

    #[test]
    fn parses_and_classifies_all_adb_endpoint_types() {
        let input = "List of devices attached\na1b2c3d4 device product:TEST-DEVICE model:TEST-DEVICE transport_id:1\n192.0.2.103:42511 device product:TEST-DEVICE model:TEST-DEVICE transport_id:3\nadb-a1b2c3d4-x._adb-tls-connect._tcp device product:TEST-DEVICE model:TEST-DEVICE transport_id:2\n";
        let devices = parse_devices(input);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].kind, EndpointKind::Usb);
        assert_eq!(devices[1].kind, EndpointKind::Wifi);
        assert_eq!(devices[2].kind, EndpointKind::Mdns);
    }

    #[test]
    fn automatic_selection_prefers_usb_over_a_saved_wifi_endpoint() {
        let mut inventory = DeviceInventory {
            endpoints: parse_devices(
                "List of devices attached\na1b2c3d4 device\n192.0.2.103:42511 device\n",
            ),
        };
        for endpoint in &mut inventory.endpoints {
            endpoint.hardware_serial = Some("a1b2c3d4".into());
        }
        let mut config = Config {
            hardware_serial: "a1b2c3d4".into(),
            selected_endpoint: Some("192.0.2.103:42511".into()),
            ..Config::default()
        };
        config.known_wifi_endpoints.push("192.0.2.103:42511".into());
        assert_eq!(
            inventory
                .resolve(&config, None)
                .expect("selection")
                .endpoint,
            "a1b2c3d4"
        );
    }

    #[test]
    fn automatic_selection_never_fails_over_to_a_wrong_hardware_serial() {
        let mut inventory = DeviceInventory {
            endpoints: parse_devices(
                "List of devices attached\n192.0.2.103:42511 device\nadb-a1b2c3d4-x._adb-tls-connect._tcp device\n",
            ),
        };
        inventory.endpoints[0].hardware_serial = Some("wrong-device".into());
        inventory.endpoints[1].hardware_serial = Some("wrong-device".into());
        let mut config = Config {
            hardware_serial: "a1b2c3d4".into(),
            ..Config::default()
        };
        config.known_wifi_endpoints.push("192.0.2.103:42511".into());
        assert_eq!(
            inventory
                .resolve(&config, None)
                .expect_err("identity mismatch")
                .kind(),
            "E_DEVICE"
        );
    }

    #[test]
    fn transport_selector_resolves_wireless_endpoint_by_exact_identity() {
        let mut inventory = DeviceInventory {
            endpoints: parse_devices("List of devices attached\n192.0.2.103:42511 device\n"),
        };
        inventory.endpoints[0].hardware_serial = Some("a1b2c3d4".into());
        let mut config = Config {
            hardware_serial: "a1b2c3d4".into(),
            ..Config::default()
        };
        config.known_wifi_endpoints.push("192.0.2.103:42511".into());
        assert_eq!(
            inventory
                .resolve(&config, Some("wireless"))
                .expect("wireless selector")
                .endpoint,
            "192.0.2.103:42511"
        );
    }

    #[test]
    fn symbolic_selector_matches_a_cached_endpoint_kind() {
        let mut endpoints = parse_devices(
            "List of devices attached\na1b2c3d4 device\nadb-a1b2c3d4-x._adb-tls-connect._tcp device\n",
        );
        assert!(super::endpoint_matches_requested(
            &endpoints[0],
            Some("USB")
        ));
        assert!(super::endpoint_matches_requested(
            &endpoints[1],
            Some("mdns")
        ));
        assert!(!super::endpoint_matches_requested(
            &endpoints[0],
            Some("MDNS")
        ));
        endpoints[0].endpoint = "other-serial".into();
        assert!(super::endpoint_matches_requested(
            &endpoints[0],
            Some("other-serial")
        ));
    }

    #[test]
    fn non_usb_cache_is_bound_to_one_live_transport_connection() {
        let cached =
            parse_devices("List of devices attached\n192.0.2.1:4321 device transport_id:7\n")
                .remove(0);
        let same =
            parse_devices("List of devices attached\n192.0.2.1:4321 device transport_id:7\n")
                .remove(0);
        let reconnected =
            parse_devices("List of devices attached\n192.0.2.1:4321 device transport_id:8\n")
                .remove(0);
        assert!(same_transport_connection(&cached, &same));
        assert!(!same_transport_connection(&cached, &reconnected));
    }

    #[test]
    fn usb_cache_is_bound_to_one_live_transport_connection() {
        let cached =
            parse_devices("List of devices attached\na1b2c3d4 device transport_id:7\n").remove(0);
        let same =
            parse_devices("List of devices attached\na1b2c3d4 device transport_id:7\n").remove(0);
        let reconnected =
            parse_devices("List of devices attached\na1b2c3d4 device transport_id:8\n").remove(0);
        assert!(same_transport_connection(&cached, &same));
        assert!(!same_transport_connection(&cached, &reconnected));
    }
}

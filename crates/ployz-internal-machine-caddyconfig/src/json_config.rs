use std::collections::BTreeMap;

use ployz_pkg_api::ServiceContainer;
use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};
use serde_json::{Map, Value, json};

use crate::caddyfile::http_upstreams_from_ports;
use crate::controller::VERIFY_PATH;

#[derive(Clone, Debug, PartialEq)]
pub struct CaddyConfig {
    apps: BTreeMap<String, Value>,
}

impl CaddyConfig {
    #[must_use]
    pub fn apps(&self) -> &BTreeMap<String, Value> {
        &self.apps
    }
}

impl Serialize for CaddyConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("apps", &self.apps)?;
        map.end()
    }
}

#[must_use]
pub fn generate_json_config(containers: &[ServiceContainer], verify_response: &str) -> CaddyConfig {
    let (http_upstreams, https_upstreams) = http_upstreams_from_ports(containers);

    let mut http_routes = routes(http_upstreams);
    http_routes.push(json!({
        "match": [{"path": [VERIFY_PATH]}],
        "handle": [{
            "body": verify_response,
            "handler": "static_response",
            "status_code": 200
        }]
    }));
    let https_routes = routes(https_upstreams);

    let mut http = Map::from_iter([
        ("listen".into(), json!([":80"])),
        ("routes".into(), Value::Array(http_routes)),
        ("logs".into(), json!({})),
    ]);
    let mut https = Map::from_iter([
        ("listen".into(), json!([":443"])),
        ("logs".into(), json!({})),
    ]);
    if !https_routes.is_empty() {
        https.insert("routes".into(), Value::Array(https_routes));
    }
    // Caddy's marshaler always emits the HTTP routes field because the verify
    // route is unconditionally present.
    debug_assert!(http.contains_key("routes"));

    let app = json!({
        "servers": {
            "http": Value::Object(std::mem::take(&mut http)),
            "https": Value::Object(std::mem::take(&mut https))
        }
    });
    CaddyConfig {
        apps: BTreeMap::from([("http".into(), app)]),
    }
}

fn routes(hosts: BTreeMap<String, Vec<String>>) -> Vec<Value> {
    hosts
        .into_iter()
        .map(|(hostname, upstreams)| {
            let upstreams = upstreams
                .into_iter()
                .map(|dial| json!({"dial": dial}))
                .collect::<Vec<_>>();
            json!({
                "match": [{"host": [hostname]}],
                "handle": [{
                    "handler": "reverse_proxy",
                    "health_checks": {
                        "passive": {"fail_duration": 30_000_000_000_i64}
                    },
                    "load_balancing": {"retries": 3},
                    "upstreams": upstreams
                }]
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use ployz_pkg_api::ServiceContainer;

    use super::*;

    fn container(ip: Option<&str>, ports: &[&str]) -> ServiceContainer {
        let value = json!({
            "Id": "test",
            "State": {"Running": true},
            "Config": {"Labels": {"uncloud.service.ports": ports.join(",")}},
            "NetworkSettings": {"Networks": {
                "uncloud": {"IPAddress": ip.unwrap_or_default()}
            }}
        });
        ServiceContainer::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    #[test]
    fn empty_config_matches_frozen_caddy_json_shape() {
        let config = generate_json_config(&[], "verification-response-body");
        assert_eq!(
            config.apps()["http"],
            json!({
                "servers": {
                    "http": {
                        "listen": [":80"],
                        "routes": [{
                            "match": [{"path": ["/.uncloud-verify"]}],
                            "handle": [{
                                "body": "verification-response-body",
                                "handler": "static_response",
                                "status_code": 200
                            }]
                        }],
                        "logs": {}
                    },
                    "https": {"listen": [":443"], "logs": {}}
                }
            })
        );
    }

    #[test]
    fn routes_are_sorted_and_retain_upstream_order() {
        let config = generate_json_config(
            &[
                container(Some("10.210.0.3"), &["b.example:8080/http"]),
                container(Some("10.210.0.2"), &["a.example:8000/https"]),
                container(Some("10.210.0.4"), &["b.example:8080/http"]),
            ],
            "id",
        );
        let servers = &config.apps()["http"]["servers"];
        assert_eq!(
            servers["http"]["routes"][0]["match"][0]["host"][0],
            "b.example"
        );
        assert_eq!(
            servers["http"]["routes"][0]["handle"][0]["upstreams"],
            json!([{"dial":"10.210.0.3:8080"}, {"dial":"10.210.0.4:8080"}])
        );
        assert_eq!(
            servers["https"]["routes"][0]["match"][0]["host"][0],
            "a.example"
        );
    }
}

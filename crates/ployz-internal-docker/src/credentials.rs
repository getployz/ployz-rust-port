use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use docker_credential::DockerCredential;
use serde::Serialize;

use crate::DockerError;

#[derive(Serialize)]
struct AuthPayload<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    identitytoken: Option<&'a str>,
    serveraddress: &'a str,
}

/// Resolve and encode non-empty credentials from the user's Docker config.
pub fn retrieve_local_registry_auth(image: &str) -> Result<Option<String>, DockerError> {
    let registry = registry_for_image(image);
    let credential = match docker_credential::get_credential(registry) {
        Ok(credential) => credential,
        Err(
            docker_credential::CredentialRetrievalError::NoCredentialConfigured
            | docker_credential::CredentialRetrievalError::ConfigNotFound
            | docker_credential::CredentialRetrievalError::ConfigReadError,
        ) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    encode_credential(registry, credential).map(Some)
}

fn encode_credential(registry: &str, credential: DockerCredential) -> Result<String, DockerError> {
    let payload = match &credential {
        DockerCredential::IdentityToken(token) => AuthPayload {
            username: None,
            password: None,
            identitytoken: Some(token),
            serveraddress: registry,
        },
        DockerCredential::UsernamePassword(username, password) => AuthPayload {
            username: Some(username),
            password: Some(password),
            identitytoken: None,
            serveraddress: registry,
        },
    };
    serde_json::to_vec(&payload)
        .map(|json| URL_SAFE.encode(json))
        .map_err(|error| DockerError::Configuration(format!("encode registry auth: {error}")))
}

fn registry_for_image(image: &str) -> &str {
    let first = image.split('/').next().unwrap_or_default();
    if image.contains('/') && (first.contains('.') || first.contains(':') || first == "localhost") {
        first
    } else {
        "index.docker.io"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_hub_and_explicit_registries_are_selected_like_docker() {
        assert_eq!(registry_for_image("alpine:latest"), "index.docker.io");
        assert_eq!(registry_for_image("library/alpine"), "index.docker.io");
        assert_eq!(registry_for_image("ghcr.io/acme/image"), "ghcr.io");
        assert_eq!(registry_for_image("localhost:5000/image"), "localhost:5000");
    }

    #[test]
    fn credentials_use_padded_url_safe_json() {
        let encoded = encode_credential(
            "registry.example",
            DockerCredential::UsernamePassword("¾".to_owned(), "secret".to_owned()),
        )
        .unwrap();
        assert!(encoded.contains('-'));
        let decoded = URL_SAFE.decode(encoded).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(value["username"], "¾");
        assert_eq!(value["serveraddress"], "registry.example");
    }

    #[test]
    fn authenticated_docker_config_credentials_are_resolved_and_encoded() {
        let config = br#"{
            "auths": {
                "registry.example": {"auth": "dXNlcjpwYXNz"}
            }
        }"#;
        let credential =
            docker_credential::get_credential_from_reader(&config[..], "registry.example").unwrap();
        let encoded = encode_credential("registry.example", credential).unwrap();
        let decoded = URL_SAFE.decode(encoded).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(value["username"], "user");
        assert_eq!(value["password"], "pass");
        assert_eq!(value["serveraddress"], "registry.example");
    }
}

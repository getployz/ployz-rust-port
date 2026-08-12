use std::collections::BTreeMap;

use oci_spec::distribution::Reference;
use ployz_internal_machine_api_pb as pb;
use serde::{Deserialize, Serialize};

use crate::{ApiError, GoMap, GoSlice, Result};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DockerImageInspect {
    #[serde(rename = "Id", alias = "ID")]
    pub id: String,
    pub repo_tags: GoSlice<String>,
    pub repo_digests: GoSlice<String>,
    pub size: i64,
    pub architecture: String,
    pub os: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DockerImageSummary {
    #[serde(rename = "Id", alias = "ID")]
    pub id: String,
    pub parent_id: String,
    pub repo_tags: GoSlice<String>,
    pub repo_digests: GoSlice<String>,
    pub created: i64,
    pub size: i64,
    pub shared_size: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub virtual_size: i64,
    pub labels: GoMap<String, String>,
    pub containers: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<OciDescriptor>,
    #[serde(default, skip_serializing_if = "GoSlice::is_empty")]
    pub manifests: GoSlice<DockerManifestSummary>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OciDescriptor {
    pub media_type: String,
    pub digest: String,
    pub size: i64,
    #[serde(default, skip_serializing_if = "GoSlice::is_empty")]
    pub urls: GoSlice<String>,
    #[serde(default, skip_serializing_if = "GoMap::is_empty")]
    pub annotations: GoMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<OciPlatform>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OciPlatform {
    pub architecture: String,
    pub os: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub os_version: String,
    #[serde(default, skip_serializing_if = "GoSlice::is_empty")]
    pub os_features: GoSlice<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub variant: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DockerManifestSize {
    pub content: i64,
    pub total: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DockerImageProperties {
    pub platform: OciPlatform,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DockerManifestSummary {
    #[serde(rename = "ID")]
    pub id: String,
    pub descriptor: OciDescriptor,
    pub available: bool,
    pub size: DockerManifestSize,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_data: Option<DockerImageProperties>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_data: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MachineImage {
    pub metadata: Option<pb::Metadata>,
    pub image: DockerImageInspect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MachineImages {
    pub metadata: Option<pb::Metadata>,
    pub images: Vec<DockerImageSummary>,
    pub containerd_store: bool,
}

impl MachineImages {
    pub fn error(&self) -> Result<()> {
        match self
            .metadata
            .as_ref()
            .map(|metadata| metadata.error.as_str())
        {
            Some(error) if !error.is_empty() => Err(ApiError::invalid(error)),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImageFilter {
    pub machines: Vec<String>,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MachineRemoteImage {
    pub metadata: Option<pb::Metadata>,
    pub image: RemoteImage,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemoteImage {
    pub reference: Option<Reference>,
    pub index_manifest: Option<serde_json::Value>,
    pub image_manifest: Option<serde_json::Value>,
}

const fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}

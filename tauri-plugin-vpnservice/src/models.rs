#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PingRequest {
    pub value: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResponse {
    pub value: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoidRequest {}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartVpnRequest {
    pub ipv4_addr: Option<String>,
    pub ipv4_addrs: Option<Vec<String>>,
    pub routes: Option<Vec<String>>,
    pub dns: Option<String>,
    pub application_mode: Option<String>,
    pub allowed_applications: Option<Vec<String>>,
    pub request_id: Option<String>,
    pub disallowed_applications: Option<Vec<String>>,
    pub mtu: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub error_msg: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VpnStatus {
    pub running: bool,
    pub application_mode: Option<String>,
    pub effective_policy: Option<EffectiveAppRoutingPolicy>,
    pub skipped_packages: Option<Vec<SkippedPackage>>,
    pub ipv4_addr: Option<String>,
    pub ipv4_addrs: Option<Vec<String>>,
    pub routes: Option<Vec<String>>,
    pub dns: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VpnTileActionResponse {
    pub action: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRoutingPolicy {
    pub schema_version: i32,
    pub mode: String,
    #[serde(default)]
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedPackage {
    pub package_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveAppRoutingPolicy {
    pub mode: String,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub uids: Vec<i32>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRoutingPolicyResult {
    pub policy: AppRoutingPolicy,
    pub effective_policy: EffectiveAppRoutingPolicy,
    #[serde(default)]
    pub skipped_packages: Vec<SkippedPackage>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAppRoutingPolicyResponse {
    pub policy: Option<AppRoutingPolicy>,
    pub effective_policy: Option<EffectiveAppRoutingPolicy>,
    #[serde(default)]
    pub skipped_packages: Vec<SkippedPackage>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAppRoutingPolicyRequest {
    pub policy: AppRoutingPolicy,
    #[serde(default)]
    pub validate_only: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    pub package_name: String,
    pub label: String,
    pub uid: i32,
    pub is_system: bool,
    pub has_launcher: bool,
    pub enabled: bool,
    pub has_internet_permission: bool,
    #[serde(default)]
    pub shared_uid_packages: Vec<String>,
    pub selectable: bool,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
    #[serde(default)]
    pub icon_key: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppCatalogResponse {
    pub self_package_name: String,
    pub self_uid: i32,
    #[serde(default)]
    pub apps: Vec<InstalledApp>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAppIconRequest {
    pub package_name: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIconResponse {
    pub package_name: String,
    #[serde(default)]
    pub icon_key: String,
    #[serde(default)]
    pub data_url: Option<String>,
}

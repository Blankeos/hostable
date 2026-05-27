use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupPolicy {
    pub enabled: bool,
    pub local_retention: RetentionPolicy,
    pub s3_retention: Option<RetentionPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub keep_last: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Config {
    pub bucket: String,
    pub region: Option<String>,
    pub endpoint_url: Option<String>,
    pub prefix: String,
    pub access_key_env: String,
    pub secret_key_env: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSpec {
    pub include_paths: Vec<PathBuf>,
    pub exclude_paths: Vec<PathBuf>,
    pub pre_copy_hook: Option<BackupHook>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackupHook {
    MinecraftRconSaveAll,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupManifest {
    pub server_id: String,
    pub adapter: String,
    pub included_paths: Vec<PathBuf>,
    pub archive_sha256: String,
}

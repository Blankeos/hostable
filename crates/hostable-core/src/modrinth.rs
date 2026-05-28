use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModrinthProjectType {
    Mod,
    Modpack,
}

impl ModrinthProjectType {
    pub fn as_facet(&self) -> &'static str {
        match self {
            ModrinthProjectType::Mod => "project_type:mod",
            ModrinthProjectType::Modpack => "project_type:modpack",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModrinthSearchResponse {
    pub hits: Vec<ModrinthSearchHit>,
    pub offset: u32,
    pub limit: u32,
    pub total_hits: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModrinthSearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub project_type: String,
    pub client_side: Option<String>,
    pub server_side: Option<String>,
    pub downloads: u64,
    pub follows: u64,
    pub icon_url: Option<String>,
    pub versions: Vec<String>,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModrinthVersion {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub files: Vec<ModrinthFile>,
    pub dependencies: Vec<ModrinthDependency>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModrinthFile {
    pub hashes: ModrinthHashes,
    pub url: String,
    pub filename: String,
    pub primary: bool,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModrinthHashes {
    pub sha1: Option<String>,
    pub sha512: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModrinthDependency {
    pub version_id: Option<String>,
    pub project_id: Option<String>,
    pub dependency_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModrinthInstallSelection {
    pub project_id: String,
    pub project_type: ModrinthProjectType,
    pub version_id: String,
    pub version_number: String,
    pub file: ModrinthInstallFile,
    pub dependencies: Vec<ModrinthInstallFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModrinthInstallFile {
    pub project_id: String,
    pub version_id: String,
    pub filename: String,
    pub url: String,
    pub sha512: Option<String>,
    pub size: u64,
}

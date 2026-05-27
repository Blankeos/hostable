use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostableConfig {
    pub admin_web: AdminWebConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminWebConfig {
    pub enabled: bool,
    pub bind_addr: String,
}

impl Default for HostableConfig {
    fn default() -> Self {
        Self {
            admin_web: AdminWebConfig {
                enabled: false,
                bind_addr: "0.0.0.0:8080".to_string(),
            },
        }
    }
}

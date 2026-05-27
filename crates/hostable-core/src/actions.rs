use crate::adapter::Protocol;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPlan {
    pub title: String,
    pub actions: Vec<HostAction>,
}

impl ActionPlan {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            actions: Vec::new(),
        }
    }

    pub fn push(mut self, action: HostAction) -> Self {
        self.actions.push(action);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostAction {
    AptInstall {
        packages: Vec<String>,
    },
    EnableAptComponent {
        components: Vec<String>,
    },
    AddArchitecture {
        arch: String,
    },
    CreateUser {
        user: String,
        system: bool,
    },
    CreateDirectory {
        path: PathBuf,
        owner: String,
    },
    Download {
        url: String,
        dest: PathBuf,
        sha256: Option<String>,
    },
    RunCommand {
        program: String,
        args: Vec<String>,
        user: Option<String>,
        cwd: Option<PathBuf>,
    },
    WriteFile {
        path: PathBuf,
        contents: String,
        owner: String,
        mode: u32,
    },
    RenderSystemdUnit {
        name: String,
        contents: String,
    },
    SystemdDaemonReload,
    SystemdEnable {
        unit: String,
    },
    SystemdStart {
        unit: String,
    },
    UfwAllow {
        port: u16,
        protocol: Protocol,
    },
}

use anyhow::{Context, Result};
use hostable_core::{
    ArkSurvivalEvolvedAdapter, BackupHook, BackupManifest, BackupSpec, ConsoleCapability,
    GameAdapter, GameKind, MinecraftAdapter, ServerRecord,
};
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct BackupRunner {
    root: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BackupRunResult {
    pub archive_path: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest: BackupManifest,
    pub remote_artifacts: Vec<RemoteBackupArtifact>,
    pub warnings: Vec<String>,
    pub pruned_local_paths: Vec<PathBuf>,
    pub pruned_remote_artifacts: Vec<RemoteBackupArtifact>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RemoteBackupArtifact {
    pub provider: String,
    pub key: String,
    pub bytes: u64,
}

impl BackupRunner {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn run_with_retention(
        &self,
        server: &ServerRecord,
        local_keep_last: u32,
    ) -> Result<BackupRunResult> {
        let spec = backup_spec(server);
        let warnings = run_pre_copy_hook(server, &spec);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock before unix epoch")?
            .as_secs();
        let backup_dir = self.root.join(&server.id.0);
        fs::create_dir_all(&backup_dir)?;

        let archive_path = backup_dir.join(format!("{}-{timestamp}.tar.zst", server.id.0));
        let archive = File::create(&archive_path)
            .with_context(|| format!("failed to create {}", archive_path.display()))?;
        let encoder = zstd::Encoder::new(archive, 3)?;
        let mut tar = tar::Builder::new(encoder);
        let included_paths = append_backup_paths(&mut tar, server, &spec)?;
        let encoder = tar.into_inner()?;
        encoder.finish()?;

        let archive_sha256 = sha256_file(&archive_path)?;
        let manifest = BackupManifest {
            server_id: server.id.0.clone(),
            adapter: format!("{:?}", server.game),
            included_paths: included_paths.clone(),
            archive_sha256,
        };
        let manifest_path = backup_dir.join(format!("{}-{timestamp}.manifest.json", server.id.0));
        fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
        let pruned_local_paths = prune_local_backups(&backup_dir, &server.id.0, local_keep_last)?;

        Ok(BackupRunResult {
            archive_path,
            manifest_path,
            manifest,
            remote_artifacts: Vec::new(),
            warnings,
            pruned_local_paths,
            pruned_remote_artifacts: Vec::new(),
        })
    }
}

fn run_pre_copy_hook(server: &ServerRecord, spec: &BackupSpec) -> Vec<String> {
    let Some(hook) = &spec.pre_copy_hook else {
        return Vec::new();
    };
    match hook {
        BackupHook::MinecraftRconSaveAll => {
            if let Err(error) = MinecraftAdapter.send_command(server, "save-all") {
                vec![format!("Minecraft save-all hook failed: {error}")]
            } else {
                Vec::new()
            }
        }
    }
}

fn backup_spec(server: &ServerRecord) -> BackupSpec {
    match server.game {
        GameKind::Minecraft => MinecraftAdapter.backup_spec(server),
        GameKind::ArkSurvivalEvolved => ArkSurvivalEvolvedAdapter.backup_spec(server),
    }
}

fn append_backup_paths(
    tar: &mut tar::Builder<zstd::Encoder<'_, File>>,
    server: &ServerRecord,
    spec: &BackupSpec,
) -> Result<Vec<PathBuf>> {
    let mut included = Vec::new();
    for path in &spec.include_paths {
        if !path.exists() {
            continue;
        }

        let archive_name = path.strip_prefix(&server.install_path).unwrap_or(path);
        if path.is_dir() {
            tar.append_dir_all(archive_name, path)
                .with_context(|| format!("failed to add {}", path.display()))?;
        } else {
            tar.append_path_with_name(path, archive_name)
                .with_context(|| format!("failed to add {}", path.display()))?;
        }
        included.push(path.clone());
    }
    Ok(included)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 16 * 1024];

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug, Clone, Default)]
struct BackupSet {
    timestamp: u64,
    archive_path: Option<PathBuf>,
    manifest_path: Option<PathBuf>,
}

fn prune_local_backups(
    backup_dir: &Path,
    server_id: &str,
    local_keep_last: u32,
) -> Result<Vec<PathBuf>> {
    let mut sets = Vec::<BackupSet>::new();
    for entry in fs::read_dir(backup_dir)
        .with_context(|| format!("failed to read {}", backup_dir.display()))?
    {
        let path = entry?.path();
        let Some((timestamp, kind)) = parse_backup_file(server_id, &path) else {
            continue;
        };
        if let Some(set) = sets.iter_mut().find(|set| set.timestamp == timestamp) {
            match kind {
                BackupFileKind::Archive => set.archive_path = Some(path),
                BackupFileKind::Manifest => set.manifest_path = Some(path),
            }
        } else {
            let mut set = BackupSet {
                timestamp,
                ..BackupSet::default()
            };
            match kind {
                BackupFileKind::Archive => set.archive_path = Some(path),
                BackupFileKind::Manifest => set.manifest_path = Some(path),
            }
            sets.push(set);
        }
    }

    sets.sort_by_key(|set| Reverse(set.timestamp));
    let mut pruned = Vec::new();
    for set in sets.into_iter().skip(local_keep_last as usize) {
        for path in [set.archive_path, set.manifest_path].into_iter().flatten() {
            if path.exists() {
                fs::remove_file(&path)
                    .with_context(|| format!("failed to prune {}", path.display()))?;
                pruned.push(path);
            }
        }
    }
    Ok(pruned)
}

#[derive(Debug, Clone, Copy)]
enum BackupFileKind {
    Archive,
    Manifest,
}

fn parse_backup_file(server_id: &str, path: &Path) -> Option<(u64, BackupFileKind)> {
    let name = path.file_name()?.to_str()?;
    let suffix = name.strip_prefix(&format!("{server_id}-"))?;
    if let Some(timestamp) = suffix.strip_suffix(".tar.zst") {
        return timestamp
            .parse()
            .ok()
            .map(|value| (value, BackupFileKind::Archive));
    }
    if let Some(timestamp) = suffix.strip_suffix(".manifest.json") {
        return timestamp
            .parse()
            .ok()
            .map(|value| (value, BackupFileKind::Manifest));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use hostable_core::{GameKind, PortSpec, Protocol, ServerId};

    #[test]
    fn creates_minecraft_archive_and_manifest() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("hostable-backup-test-{unique}"));
        let server_root = root.join("server");
        let backup_root = root.join("backups");
        fs::create_dir_all(server_root.join("world")).unwrap();
        fs::write(server_root.join("world/level.dat"), b"world").unwrap();
        fs::write(server_root.join("server.properties"), b"motd=test").unwrap();
        fs::write(server_root.join("hostable.minecraft.mods.json"), b"{}").unwrap();

        let server = ServerRecord {
            id: ServerId("friends".to_string()),
            name: "Friends".to_string(),
            game: GameKind::Minecraft,
            install_path: server_root,
            service_name: "hostable-friends.service".to_string(),
            ports: vec![PortSpec {
                port: 25565,
                protocol: Protocol::Tcp,
                label: "Minecraft server".to_string(),
            }],
            adapter_config: serde_json::json!({}),
        };

        let result = BackupRunner::new(backup_root)
            .run_with_retention(&server, u32::MAX)
            .unwrap();

        assert!(result.archive_path.exists());
        assert!(result.manifest_path.exists());
        assert_eq!(result.manifest.server_id, "friends");
        assert!(
            result
                .manifest
                .included_paths
                .iter()
                .any(|path| path.ends_with("world"))
        );
    }

    #[test]
    fn prunes_local_backups_by_retention_count() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("hostable-prune-test-{unique}"));
        let backup_root = root.join("backups");
        let backup_dir = backup_root.join("friends");
        fs::create_dir_all(&backup_dir).unwrap();
        for timestamp in [1, 2, 3] {
            fs::write(
                backup_dir.join(format!("friends-{timestamp}.tar.zst")),
                b"archive",
            )
            .unwrap();
            fs::write(
                backup_dir.join(format!("friends-{timestamp}.manifest.json")),
                b"{}",
            )
            .unwrap();
        }

        let pruned = prune_local_backups(&backup_dir, "friends", 2).unwrap();

        assert_eq!(pruned.len(), 2);
        assert!(backup_dir.join("friends-3.tar.zst").exists());
        assert!(backup_dir.join("friends-2.tar.zst").exists());
        assert!(!backup_dir.join("friends-1.tar.zst").exists());

        let _ = fs::remove_dir_all(root);
    }
}

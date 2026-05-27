use anyhow::{Context, Result};
use hostable_core::{
    ArkSurvivalEvolvedAdapter, BackupManifest, BackupSpec, GameAdapter, GameKind, MinecraftAdapter,
    ServerRecord,
};
use sha2::{Digest, Sha256};
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
}

impl BackupRunner {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn run(&self, server: &ServerRecord) -> Result<BackupRunResult> {
        let spec = backup_spec(server);
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

        Ok(BackupRunResult {
            archive_path,
            manifest_path,
            manifest,
        })
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

        let result = BackupRunner::new(backup_root).run(&server).unwrap();

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
}

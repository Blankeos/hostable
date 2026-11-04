//! Day-two administration of a Minecraft server: mods and plugins on disk,
//! whitelist/ops files and the `list` reply. Everything here is plain file and
//! string handling so it can be tested without a running server.

use crate::{HostableError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const SIDECAR: &str = "hostable.installed-mods.json";
const MAX_NAME_LEN: usize = 16;

/// Fabric ships `fabric-server-launch.jar`, but the launcher download from fabricmc.net
/// (and most modpacks) is named like `fabric-server-mc.1.21.1-loader.0.18.4-launcher.1.1.2.jar`.
pub fn has_fabric_launcher(path: &Path) -> bool {
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".jar") && name.starts_with("fabric-server-")
        })
}

/// Directory that holds mods (Fabric) or plugins (Paper). `None` for vanilla,
/// which cannot load either.
pub fn extensions_dir(install_path: &Path, server_type: &str) -> Option<PathBuf> {
    match server_type {
        "fabric" => Some(install_path.join("mods")),
        "paper" => Some(install_path.join("plugins")),
        _ => None,
    }
}

/// "mod" for Fabric, "plugin" for Paper.
pub fn extension_noun(server_type: &str) -> &'static str {
    if server_type == "paper" {
        "plugin"
    } else {
        "mod"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledMod {
    pub filename: String,
    pub enabled: bool,
    pub size: u64,
    /// Set when Hostable installed the file from Modrinth.
    pub title: Option<String>,
    pub version: Option<String>,
    pub project_id: Option<String>,
    /// Part of the server's modpack, as opposed to a mod added on top.
    #[serde(default)]
    pub in_pack: bool,
}

/// The modpack a server was built from, read from the files the pack left behind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModpackInfo {
    pub name: String,
    pub version: Option<String>,
    /// Mod jars the pack ships. `None` when only the pack's name is known.
    pub files: Option<Vec<String>>,
}

/// Understands `modpack.json` (name + file list, used by packs such as Cobbleverse),
/// `modrinth.index.json` (a Modrinth pack) and Hostable's own record of a pack it installed.
pub fn detect_modpack(install_path: &Path) -> Option<ModpackInfo> {
    let read = |name: &str| -> Option<serde_json::Value> {
        serde_json::from_str(&fs::read_to_string(install_path.join(name)).ok()?).ok()
    };
    let mod_jars = |paths: Vec<&str>| -> Vec<String> {
        let mut jars: Vec<String> = paths
            .into_iter()
            .filter_map(|path| path.strip_prefix("mods/"))
            .filter(|name| !name.contains('/'))
            .map(|name| enabled_name(name).to_string())
            .collect();
        jars.sort();
        jars
    };
    let text = |value: &serde_json::Value, key: &str| -> Option<String> {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    if let Some(pack) = read("modpack.json")
        && let Some(name) = text(&pack, "modPackName")
    {
        let files = pack.get("file").and_then(|f| f.as_object());
        return Some(ModpackInfo {
            name,
            version: text(&pack, "modPackVersion"),
            files: files.map(|files| mod_jars(files.keys().map(String::as_str).collect())),
        });
    }
    if let Some(pack) = read("modrinth.index.json")
        && let Some(name) = text(&pack, "name")
    {
        let paths: Vec<&str> = pack
            .get("files")
            .and_then(|f| f.as_array())
            .map(|files| {
                files
                    .iter()
                    .filter_map(|file| file.get("path").and_then(|p| p.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        return Some(ModpackInfo {
            name,
            version: text(&pack, "versionId"),
            files: Some(mod_jars(paths)),
        });
    }
    let record = read("hostable.minecraft.mods.json")?;
    let pack = record.get("modpack")?;
    Some(ModpackInfo {
        name: text(pack, "title")?,
        version: text(pack, "version_number"),
        files: None,
    })
}

/// Flag which mods belong to the pack. With no file list, anything Hostable did not
/// install on its own from Modrinth counts as part of the pack.
pub fn mark_pack_mods(mods: &mut [InstalledMod], pack: &ModpackInfo) {
    for item in mods {
        item.in_pack = match &pack.files {
            Some(files) => files
                .binary_search_by(|name| name.as_str().cmp(enabled_name(&item.filename)))
                .is_ok(),
            None => item.project_id.is_none(),
        };
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledRecord {
    pub project_id: String,
    pub title: String,
    pub version: String,
}

type Sidecar = BTreeMap<String, InstalledRecord>;

fn read_sidecar(install_path: &Path) -> Sidecar {
    fs::read_to_string(install_path.join(SIDECAR))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_sidecar(install_path: &Path, sidecar: &Sidecar) -> Result<()> {
    let text = serde_json::to_string_pretty(sidecar)
        .map_err(|error| HostableError::InvalidInput(error.to_string()))?;
    fs::write(install_path.join(SIDECAR), text)?;
    Ok(())
}

/// Remember which Modrinth project a downloaded file came from.
pub fn record_install(install_path: &Path, filename: &str, record: InstalledRecord) -> Result<()> {
    let mut sidecar = read_sidecar(install_path);
    sidecar.insert(filename.to_string(), record);
    write_sidecar(install_path, &sidecar)
}

/// Mods are plain files in one directory; reject anything that is not a bare
/// `name.jar` or `name.jar.disabled` so API callers cannot escape it.
pub fn check_mod_filename(filename: &str) -> Result<()> {
    let ok_chars = filename.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '+' | ' ' | '(' | ')')
    });
    let ok_shape = filename.ends_with(".jar") || filename.ends_with(".jar.disabled");
    if filename.is_empty()
        || filename.len() > 200
        || filename.starts_with('.')
        || filename.contains("..")
        || !ok_chars
        || !ok_shape
    {
        return Err(HostableError::InvalidInput(format!(
            "'{filename}' is not a valid mod file name"
        )));
    }
    Ok(())
}

fn enabled_name(filename: &str) -> &str {
    filename.strip_suffix(".disabled").unwrap_or(filename)
}

pub fn list_mods(install_path: &Path, dir: &Path) -> Result<Vec<InstalledMod>> {
    let sidecar = read_sidecar(install_path);
    let mut mods = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(mods),
        Err(error) => return Err(error.into()),
    };
    for entry in entries.flatten() {
        let filename = entry.file_name().to_string_lossy().to_string();
        if check_mod_filename(&filename).is_err() || !entry.path().is_file() {
            continue;
        }
        let record = sidecar.get(enabled_name(&filename));
        mods.push(InstalledMod {
            enabled: !filename.ends_with(".disabled"),
            size: entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            title: record.map(|record| record.title.clone()),
            version: record.map(|record| record.version.clone()),
            project_id: record.map(|record| record.project_id.clone()),
            in_pack: false,
            filename,
        });
    }
    mods.sort_by_key(display_key);
    Ok(mods)
}

fn display_key(item: &InstalledMod) -> String {
    item.title
        .clone()
        .unwrap_or_else(|| enabled_name(&item.filename).to_string())
        .to_lowercase()
}

/// Rename `x.jar` <-> `x.jar.disabled`. Servers only load `*.jar`.
pub fn set_mod_enabled(dir: &Path, filename: &str, enabled: bool) -> Result<String> {
    check_mod_filename(filename)?;
    let base = enabled_name(filename).to_string();
    let from = dir.join(filename);
    let to_name = if enabled {
        base
    } else {
        format!("{base}.disabled")
    };
    if !from.is_file() {
        return Err(HostableError::InvalidInput(format!("{filename} not found")));
    }
    if to_name != filename {
        fs::rename(&from, dir.join(&to_name))?;
    }
    Ok(to_name)
}

pub fn remove_mod(install_path: &Path, dir: &Path, filename: &str) -> Result<()> {
    check_mod_filename(filename)?;
    let path = dir.join(filename);
    if !path.is_file() {
        return Err(HostableError::InvalidInput(format!("{filename} not found")));
    }
    fs::remove_file(path)?;
    let mut sidecar = read_sidecar(install_path);
    if sidecar.remove(enabled_name(filename)).is_some() {
        write_sidecar(install_path, &sidecar)?;
    }
    Ok(())
}

/// Minecraft account names: up to 16 letters, digits or underscores. Offline
/// servers are looser in theory but this keeps console input safe.
pub fn valid_player_name(name: &str) -> bool {
    (1..=MAX_NAME_LEN).contains(&name.len())
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerEntry {
    pub name: String,
    /// Operator permission level (1-4), only for ops.json.
    pub level: Option<u8>,
    /// False for names that look like terminal garbage or are otherwise
    /// unsafe to feed back into a command.
    pub clean: bool,
}

/// Read whitelist.json / ops.json / banned-players.json. Missing file = empty.
pub fn read_player_file(path: &Path) -> Vec<PlayerEntry> {
    #[derive(Deserialize)]
    struct Raw {
        name: Option<String>,
        level: Option<u8>,
    }
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let raw: Vec<Raw> = serde_json::from_str(&text).unwrap_or_default();
    let mut entries: Vec<PlayerEntry> = raw
        .into_iter()
        .filter_map(|item| {
            let name = item.name?;
            let clean = valid_player_name(&name);
            Some(PlayerEntry {
                name: sanitize_text(&name),
                level: item.level,
                clean,
            })
        })
        .collect();
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    entries
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OnlinePlayers {
    pub online: u32,
    pub max: Option<u32>,
    pub names: Vec<String>,
}

/// Parse "There are 3 of a max of 20 players online: a, b, c".
pub fn parse_list_reply(reply: &str) -> Option<OnlinePlayers> {
    let reply = sanitize_text(reply);
    let rest = reply.split("There are ").nth(1)?;
    let mut words = rest.split_whitespace();
    let online = words.next()?.parse().ok()?;
    let max = rest
        .split("max of ")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|word| word.parse().ok());
    let names = rest
        .split_once(':')
        .map(|(_, names)| {
            names
                .split(',')
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Some(OnlinePlayers { online, max, names })
}

/// Drop ANSI escape sequences and control characters (keeps tab and newline).
/// Console output and imported files may contain them; a terminal UI must
/// never pass them through.
pub fn sanitize_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for next in chars.by_ref() {
                        if next.is_ascii_alphabetic() || next == '~' {
                            break;
                        }
                    }
                } else {
                    chars.next();
                }
            }
            // Minecraft colour codes.
            '\u{a7}' => {
                chars.next();
            }
            '\n' | '\t' => out.push(ch),
            ch if ch.is_control() => {}
            ch => out.push(ch),
        }
    }
    out
}

/// Minecraft version of an existing server: from the record when Hostable
/// installed it, otherwise from the startup lines in `logs/latest.log`.
pub fn detect_game_version(install_path: &Path, recorded: Option<&str>) -> Option<String> {
    if let Some(version) = recorded.filter(|v| !v.is_empty() && *v != "latest") {
        return Some(version.to_string());
    }
    let log = fs::read_to_string(install_path.join("logs/latest.log")).ok()?;
    log.lines().find_map(|line| {
        for marker in ["Starting minecraft server version ", "Loading Minecraft "] {
            if let Some(rest) = line.split(marker).nth(1) {
                let version = rest.split_whitespace().next()?;
                if version.chars().all(|ch| ch.is_ascii_digit() || ch == '.') {
                    return Some(version.to_string());
                }
            }
        }
        None
    })
}

/// Properties from server.properties that a read-only overview needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ServerProperties {
    pub motd: Option<String>,
    pub max_players: Option<u32>,
    pub white_list: bool,
    pub online_mode: bool,
    pub difficulty: Option<String>,
    pub gamemode: Option<String>,
    pub level_name: Option<String>,
}

pub fn read_properties(install_path: &Path) -> ServerProperties {
    let text = fs::read_to_string(install_path.join("server.properties")).unwrap_or_default();
    let get = |key: &str| {
        text.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name.trim() == key).then(|| value.trim().to_string())
        })
    };
    ServerProperties {
        motd: get("motd").map(|value| sanitize_text(&value)),
        max_players: get("max-players").and_then(|value| value.parse().ok()),
        white_list: get("white-list").as_deref() == Some("true"),
        online_mode: get("online-mode").as_deref() != Some("false"),
        difficulty: get("difficulty"),
        gamemode: get("gamemode"),
        level_name: get("level-name"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hostable-mcadmin-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn filenames_cannot_escape_the_mods_directory() {
        assert!(check_mod_filename("lithium-0.12.jar").is_ok());
        assert!(check_mod_filename("lithium-0.12.jar.disabled").is_ok());
        assert!(check_mod_filename("../evil.jar").is_err());
        assert!(check_mod_filename("a/b.jar").is_err());
        assert!(check_mod_filename(".hidden.jar").is_err());
        assert!(check_mod_filename("notes.txt").is_err());
        assert!(check_mod_filename("").is_err());
    }

    #[test]
    fn toggling_a_mod_renames_it_and_lists_state() {
        let root = temp_dir("toggle");
        let mods = root.join("mods");
        fs::create_dir_all(&mods).unwrap();
        fs::write(mods.join("sodium.jar"), b"x").unwrap();
        fs::write(mods.join("README.txt"), b"x").unwrap();
        record_install(
            &root,
            "sodium.jar",
            InstalledRecord {
                project_id: "AANobbMI".into(),
                title: "Sodium".into(),
                version: "0.6".into(),
            },
        )
        .unwrap();

        let name = set_mod_enabled(&mods, "sodium.jar", false).unwrap();
        assert_eq!(name, "sodium.jar.disabled");
        let listed = list_mods(&root, &mods).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].enabled);
        assert_eq!(listed[0].title.as_deref(), Some("Sodium"));

        set_mod_enabled(&mods, "sodium.jar.disabled", true).unwrap();
        assert!(mods.join("sodium.jar").exists());

        remove_mod(&root, &mods, "sodium.jar").unwrap();
        assert!(list_mods(&root, &mods).unwrap().is_empty());
        assert!(read_sidecar(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn player_files_are_read_and_garbage_names_are_flagged() {
        let root = temp_dir("players");
        fs::write(
            root.join("ops.json"),
            "[{\"name\":\"Blankeos\",\"level\":4},{\"name\":\"blankeos\\u001b[d\\u001bb\",\"level\":4}]",
        )
        .unwrap();
        let ops = read_player_file(&root.join("ops.json"));
        assert_eq!(ops.len(), 2);
        assert!(ops.iter().any(|op| op.name == "Blankeos" && op.clean));
        let garbage = ops.iter().find(|op| !op.clean).unwrap();
        assert!(!garbage.name.contains('\u{1b}'));
        assert!(read_player_file(&root.join("missing.json")).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn player_names_are_validated() {
        assert!(valid_player_name("Blankeos"));
        assert!(valid_player_name("a_b_9"));
        assert!(!valid_player_name(""));
        assert!(!valid_player_name("has space"));
        assert!(!valid_player_name("semi;colon"));
        assert!(!valid_player_name("seventeen_chars_xx"));
    }

    #[test]
    fn list_reply_is_parsed() {
        let reply = "There are 2 of a max of 20 players online: Blankeos, Talaba";
        let players = parse_list_reply(reply).unwrap();
        assert_eq!(players.online, 2);
        assert_eq!(players.max, Some(20));
        assert_eq!(players.names, vec!["Blankeos", "Talaba"]);
        let empty = parse_list_reply("There are 0 of a max of 20 players online: ").unwrap();
        assert_eq!(empty.online, 0);
        assert!(empty.names.is_empty());
        assert!(parse_list_reply("Unknown command").is_none());
    }

    #[test]
    fn sanitize_strips_escapes_and_colour_codes() {
        assert_eq!(sanitize_text("a\u{1b}[31mred\u{1b}[0m"), "ared");
        assert_eq!(sanitize_text("\u{a7}aGreen"), "Green");
        assert_eq!(sanitize_text("ok\u{7}\nline"), "ok\nline");
    }

    #[test]
    fn game_version_comes_from_the_record_or_the_log() {
        let root = temp_dir("version");
        assert_eq!(
            detect_game_version(&root, Some("1.21.1")).as_deref(),
            Some("1.21.1")
        );
        assert_eq!(detect_game_version(&root, None), None);
        fs::create_dir_all(root.join("logs")).unwrap();
        fs::write(
            root.join("logs/latest.log"),
            "[12:00:00] [main/INFO]: Loading Minecraft 1.21.1 with Fabric Loader 0.17.2\n",
        )
        .unwrap();
        assert_eq!(
            detect_game_version(&root, Some("latest")).as_deref(),
            Some("1.21.1")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn properties_are_read() {
        let root = temp_dir("props");
        fs::write(
            root.join("server.properties"),
            "motd=Hi\nmax-players=12\nwhite-list=false\nonline-mode=false\n",
        )
        .unwrap();
        let props = read_properties(&root);
        assert_eq!(props.max_players, Some(12));
        assert!(!props.white_list);
        assert!(!props.online_mode);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_both_fabric_launcher_names() {
        for name in [
            "fabric-server-launch.jar",
            "fabric-server-mc.1.21.1-loader.0.18.4-launcher.1.1.2.jar",
        ] {
            let root = temp_dir("fabric-launcher");
            fs::write(root.join(name), b"").unwrap();
            assert!(has_fabric_launcher(&root), "{name}");
        }
        let root = temp_dir("fabric-launcher-none");
        fs::write(root.join("server.jar"), b"").unwrap();
        assert!(!has_fabric_launcher(&root));
    }

    #[test]
    fn reads_modpack_json_and_flags_pack_mods() {
        let root = temp_dir("modpack");
        fs::write(
            root.join("modpack.json"),
            r#"{"modPackName":"COBBLEVERSE","modPackVersion":"1.7.42","file":{"mods/a.jar":{},"config/x.toml":{},"mods/b.jar":{}}}"#,
        )
        .unwrap();
        let pack = detect_modpack(&root).unwrap();
        assert_eq!(pack.name, "COBBLEVERSE");
        assert_eq!(pack.version.as_deref(), Some("1.7.42"));
        assert_eq!(
            pack.files.as_deref(),
            Some(&["a.jar".to_string(), "b.jar".to_string()][..])
        );
        let item = |filename: &str| InstalledMod {
            filename: filename.to_string(),
            enabled: true,
            size: 0,
            title: None,
            version: None,
            project_id: None,
            in_pack: false,
        };
        let mut mods = vec![item("a.jar"), item("b.jar.disabled"), item("mine.jar")];
        mark_pack_mods(&mut mods, &pack);
        assert_eq!(
            mods.iter().map(|m| m.in_pack).collect::<Vec<_>>(),
            [true, true, false]
        );
    }
}

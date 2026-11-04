//! Input validation for everything that ends up in root-owned files
//! (systemd units, `server.properties`, INI files) or in `chown`/`tar` paths.
//!
//! The daemon runs as root, so these checks are the trust boundary between
//! "an API caller" and "files that root loads".

use hostable_core::CreateServerInput;
use std::path::{Component, Path, PathBuf};

pub const SERVERS_ROOT: &str = "/srv/hostable/servers";

/// Directories that must never be adopted as a server folder.
const DENIED_IMPORT_ROOTS: &[&str] = &[
    "/", "/bin", "/boot", "/dev", "/etc", "/lib", "/lib64", "/proc", "/root", "/run", "/sbin",
    "/sys", "/usr", "/var", "/home",
];

pub fn validate_create_input(
    input: &CreateServerInput,
    servers_root: &Path,
) -> Result<(), &'static str> {
    validate_name(&input.name)?;
    validate_managed_path(&input.install_path, servers_root)?;
    if let Some(memory_mb) = input.memory_mb
        && !(512..=262_144).contains(&memory_mb)
    {
        return Err("memory_mb must be between 512 and 262144");
    }
    for field in ["game_version", "loader_version", "build_version"] {
        let value = match field {
            "game_version" => input.game_version.as_deref(),
            "loader_version" => input.loader_version.as_deref(),
            _ => input.build_version.as_deref(),
        };
        if let Some(value) = value
            && !is_version_like(value)
        {
            return Err("version fields may only contain letters, digits, '.', '-', '_' and '+'");
        }
    }
    if let Some(extra) = input.extra.as_object() {
        for (key, value) in extra {
            if let Some(text) = value.as_str() {
                if has_control_chars(text) {
                    return Err("extra settings must not contain control characters");
                }
                // These values end up inside shell scripts and quoted INI values.
                if text.contains(['"', '`', '$', '\\']) {
                    return Err("extra settings must not contain \" ` $ or backslash characters");
                }
                if matches!(key.as_str(), "rcon_password" | "admin_password")
                    && !is_secret_like(text)
                {
                    return Err(
                        "passwords may only contain letters, digits, '-' and '_' (8-128 chars)",
                    );
                }
            }
        }
    }
    Ok(())
}

pub fn validate_name(name: &str) -> Result<(), &'static str> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 48 {
        return Err("server name must be 1-48 characters");
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
    {
        return Err("server name may only contain letters, digits, spaces, '-', '_' and '.'");
    }
    if !trimmed.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err("server name needs at least one letter or digit");
    }
    Ok(())
}

/// A path for a server Hostable creates itself: it must live directly under
/// the managed servers root.
pub fn validate_managed_path(path: &Path, servers_root: &Path) -> Result<(), &'static str> {
    let normalized =
        normalize_absolute(path).ok_or("install_path must be absolute without '..'")?;
    let root = normalize_absolute(servers_root).ok_or("invalid servers root")?;
    if normalized == root || !normalized.starts_with(&root) {
        return Err("install_path must be a folder inside /srv/hostable/servers");
    }
    reject_symlinks(&normalized, &root)
}

/// A path for an existing server the user wants Hostable to adopt.
pub fn validate_import_path(path: &Path) -> Result<PathBuf, &'static str> {
    let normalized =
        normalize_absolute(path).ok_or("install_path must be absolute without '..'")?;
    if DENIED_IMPORT_ROOTS
        .iter()
        .any(|denied| normalized == Path::new(denied))
    {
        return Err("refusing to manage a system directory");
    }
    if normalized.components().count() < 3 {
        return Err("install_path is too shallow to be a server folder");
    }
    let canonical = normalized
        .canonicalize()
        .map_err(|_| "install_path does not exist")?;
    if !canonical.is_dir() {
        return Err("install_path must be a directory");
    }
    if DENIED_IMPORT_ROOTS
        .iter()
        .any(|denied| canonical == Path::new(denied))
    {
        return Err("refusing to manage a system directory");
    }
    Ok(canonical)
}

pub fn is_safe_service_name(name: &str) -> bool {
    name.strip_prefix("hostable-")
        .and_then(|rest| rest.strip_suffix(".service"))
        .is_some_and(|slug| {
            !slug.is_empty()
                && slug
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
}

fn normalize_absolute(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => out.push("/"),
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// Walk from the servers root towards `path` and refuse symlinked components,
/// since root later chowns / archives this tree.
fn reject_symlinks(path: &Path, root: &Path) -> Result<(), &'static str> {
    let mut current = root.to_path_buf();
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "path escapes servers root")?;
    for component in relative.components() {
        current.push(component);
        if let Ok(meta) = std::fs::symlink_metadata(&current)
            && meta.file_type().is_symlink()
        {
            return Err("install_path must not contain symlinks");
        }
    }
    Ok(())
}

fn has_control_chars(value: &str) -> bool {
    value.chars().any(|c| c.is_control())
}

fn is_version_like(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

fn is_secret_like(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hostable_core::ServerType;

    fn input(name: &str, path: &str) -> CreateServerInput {
        CreateServerInput {
            name: name.to_string(),
            install_path: PathBuf::from(path),
            server_type: ServerType::Vanilla,
            game_version: Some("1.20.4".to_string()),
            loader_version: None,
            build_version: None,
            memory_mb: Some(2048),
            accepted_license: true,
            extra: serde_json::json!({ "motd": "hello", "rcon_password": "rcon-1234abcd" }),
        }
    }

    #[test]
    fn accepts_a_normal_input() {
        let ok = input("Friends SMP", "/srv/hostable/servers/friends-smp");
        assert!(validate_create_input(&ok, Path::new(SERVERS_ROOT)).is_ok());
    }

    #[test]
    fn rejects_newline_injection_in_names() {
        let bad = input("a\n[Service]\nExecStart=/bin/sh", "/srv/hostable/servers/a");
        assert!(validate_create_input(&bad, Path::new(SERVERS_ROOT)).is_err());
    }

    #[test]
    fn rejects_paths_outside_the_servers_root() {
        for path in [
            "/etc",
            "/srv/hostable/servers",
            "/srv/hostable/servers/../../etc",
            "/srv/hostable/other",
            "relative/path",
        ] {
            let bad = input("ok", path);
            assert!(
                validate_create_input(&bad, Path::new(SERVERS_ROOT)).is_err(),
                "{path} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_control_chars_and_weak_secrets() {
        let mut bad = input("ok", "/srv/hostable/servers/ok");
        bad.extra = serde_json::json!({ "motd": "line1\nrcon.password=x" });
        assert!(validate_create_input(&bad, Path::new(SERVERS_ROOT)).is_err());

        bad.extra = serde_json::json!({ "rcon_password": "x y\n" });
        assert!(validate_create_input(&bad, Path::new(SERVERS_ROOT)).is_err());
    }

    #[test]
    fn rejects_shell_metacharacters_in_extras() {
        for bad in ["a\"; rm -rf /; \"", "$(reboot)", "`id`", "back\\slash"] {
            let mut input = input("ok", "/srv/hostable/servers/ok");
            input.extra = serde_json::json!({ "session_name": bad });
            assert!(
                validate_create_input(&input, Path::new(SERVERS_ROOT)).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn import_refuses_system_directories() {
        for path in ["/", "/etc", "/usr", "/home", "../x"] {
            assert!(validate_import_path(Path::new(path)).is_err(), "{path}");
        }
    }

    #[test]
    fn service_names_are_derived_only() {
        assert!(is_safe_service_name("hostable-friends-smp.service"));
        assert!(!is_safe_service_name("sshd.service"));
        assert!(!is_safe_service_name("hostable-.service"));
        assert!(!is_safe_service_name("hostable-a;b.service"));
    }
}

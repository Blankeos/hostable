# Hostable Plan

Hostable is a Rust TUI plus a tiny password-gated web admin for creating and managing self-hosted Minecraft and ARK servers on a VPS. The target user is a casual friend group that wants to buy a VPS, run one install command, create a server, and avoid learning the whole Linux game-server setup by hand.

This plan is based on:

- ARK VPS flow: https://www.carlo.tl/blog/host-an-ark-server-on-a-vps
- Minecraft VPS flow: https://www.carlo.tl/blog/host-a-minecraft-server-on-a-vps
- Solid Hop stack: https://github.com/blankeos/solid-hop
- Modrinth API docs: https://docs.modrinth.com/api/

## Product Shape

Ship three user-facing apps:

- `hostable`: the Ratatui TUI for installation, server creation, importing existing servers, config editing, backups, logs, and settings.
- `hostabled`: a local Rust daemon that owns privileged operations, process control, manual backup jobs, the HTTP API, and the bundled admin frontend.
- `web/landing`: a Solid Hop landing page for Cloudflare deployment.

The TUI should not directly become the process supervisor. The web admin needs the same start/stop/status controls, installs are long-running, and backups should be executed by one durable local service, so `hostabled` should own server state. The TUI becomes the full local operator interface.

No Docker for MVP. Hostable should manage normal Linux users, directories, systemd units, firewall ports, and files directly.

## Target Platform

MVP target:

- Debian 12/13 and Ubuntu 24.04 style VPSes, both supported best effort.
- x86_64 first.
- systemd.
- UFW support when present.
- One shared `hostable` system user by default.
- Game data under `/srv/hostable/servers/<server-id>/`.

Initial install layout:

```txt
/usr/local/bin/hostable
/usr/local/bin/hostabled
/etc/hostable/config.toml
/etc/hostable/secrets.env
/var/lib/hostable/registry.sqlite
/var/lib/hostable/web-admin/
/var/log/hostable/
/srv/hostable/servers/<server-id>/
```

Use systemd services:

```txt
hostabled.service
hostable-<server-id>.service
```

Backup timers are a later enhancement. MVP backups are manually triggered from the TUI.

## MVP Happy Path

The first successful user journey should be:

1. User starts with a fresh Debian or Ubuntu VPS and SSH access.
2. User SSHes into the VPS and runs the install command.
3. Installer downloads the latest GitHub Release artifact, installs `hostable` and `hostabled`, creates the service user, writes systemd units, and starts `hostabled`.
4. User runs `hostable`.
5. TUI opens.
6. User chooses `Create new server`.
7. Wizard asks for Minecraft or ARK.
8. User chooses Minecraft first for the golden path.
9. Wizard asks for server name, install folder, memory, Minecraft server type, Minecraft/Fabric/Paper versions as needed, license acceptance, compatible Modrinth options when available, and server settings.
10. Hostable installs dependencies, creates files, opens local VPS firewall ports, writes the systemd service, starts the server, and shows the join address.
11. User can start/stop/restart from the TUI.

Minecraft is the first end-to-end proof. ARK should still be implemented against the same adapter structure for MVP, not left as a vague future feature.

## Privilege Model

The install command can be frictionless and permissive because this is usually a fresh VPS, but Hostable should create the safer structure underneath:

- Installer runs with root/sudo.
- `hostabled` has enough privilege to install packages, write systemd units, manage UFW, manage server files, and control services.
- Game servers run as a non-root `hostable` user by default.
- The `hostable` TUI talks to `hostabled` over a local Unix socket.
- Users should not manually create Linux users, directories, permissions, or systemd units.

## License Acceptance

Do not silently accept third-party licenses. The create-server wizard should require explicit confirmation before automation continues:

- Minecraft: user accepts the Minecraft EULA, then Hostable writes `eula.txt`.
- ARK/SteamCMD: user accepts the Steam/SteamCMD-related requirements needed to install the server.

Store the acceptance timestamp and adapter kind in SQLite so retries do not keep nagging for the same server.

## Networking Boundary

Hostable should automate the networking steps that live inside the VPS:

- Detect public IP.
- Install/configure UFW if needed.
- Always allow SSH before enabling UFW.
- Open game ports automatically.
- Validate that the server process is listening on the expected local port.
- Show the join address.
- Provide a network doctor screen with pass/fail checks.

Hostable should not automate Cloudflare or DNS in MVP. If local checks pass but external connectivity fails, Hostable should tell the user: "Your VPS provider may have an external firewall. Open these ports in your VPS dashboard: ...".

## Rust Workspace

Proposed structure:

```txt
Cargo.toml
crates/
  hostable-core/
    src/
      adapter.rs
      actions.rs
      backup.rs
      config.rs
      error.rs
      registry.rs
      service.rs
      games/
        minecraft.rs
        ark_se.rs
  hostabled/
    src/
      api/
      auth.rs
      jobs.rs
      main.rs
      static_web.rs
  hostable-tui/
    src/
      app.rs
      main.rs
      screens/
      widgets/
  hostable-cli/
    src/main.rs
web/
  admin/
  landing/
```

`hostable-cli` is optional but useful for scripts and debugging. It can expose commands such as `hostablectl status`, `hostablectl backup <id>`, and `hostablectl doctor`.

Keep adapters compiled in for the first version. Dynamic plugins are not needed yet. The important part is that adding a new self-hosted game means implementing one adapter module and registering it.

## Core Model

Canonical records:

```rust
struct ServerRecord {
    id: ServerId,
    name: String,
    game: GameKind,
    install_path: PathBuf,
    service_name: String,
    ports: Vec<PortSpec>,
    created_at: DateTime<Utc>,
    adapter_config: serde_json::Value,
}

struct BackupPolicy {
    enabled: bool,
    local_retention: RetentionPolicy,
    s3_retention: Option<RetentionPolicy>,
}

struct S3Config {
    bucket: String,
    region: Option<String>,
    endpoint_url: Option<String>,
    prefix: String,
    access_key_env: String,
    secret_key_env: String,
}
```

Use SQLite for the registry, events, jobs, and backup history. Keep game-owned config in the real game files so users are never locked into Hostable.

## Adapter Contract

The adapter should describe what Hostable needs to know about a game without hardcoding game-specific logic into the TUI or daemon.

```rust
#[async_trait::async_trait]
pub trait GameAdapter: Send + Sync {
    fn kind(&self) -> GameKind;
    fn display_name(&self) -> &'static str;
    fn default_ports(&self) -> Vec<PortSpec>;
    fn requirements(&self) -> Requirements;

    async fn detect_existing(&self, path: &Path) -> Result<DetectedServer>;
    async fn plan_install(&self, input: CreateServerInput) -> Result<ActionPlan>;
    async fn plan_import(&self, detected: DetectedServer) -> Result<ActionPlan>;

    async fn read_config(&self, server: &ServerRecord) -> Result<ConfigDocument>;
    async fn write_config(&self, server: &ServerRecord, patch: ConfigPatch) -> Result<()>;

    fn backup_spec(&self, server: &ServerRecord) -> BackupSpec;
    async fn graceful_start(&self, server: &ServerRecord) -> Result<()>;
    async fn graceful_stop(&self, server: &ServerRecord) -> Result<()>;
    async fn health_check(&self, server: &ServerRecord) -> Result<HealthStatus>;

    fn console(&self) -> Option<&dyn ConsoleCapability> {
        None
    }

    fn modrinth(&self) -> Option<&dyn ModrinthCapability> {
        None
    }
}
```

The TUI should show game-specific screens based on adapter capabilities. Minecraft exposes `ModrinthCapability`; ARK does not. Minecraft and ARK expose `ConsoleCapability` through RCON so the TUI can run admin commands from a server `Console` tab. This keeps game-specific logic in adapters without hardcoding Minecraft-only or ARK-only UI into the core server screens.

Adapters should emit typed actions instead of raw shell scripts wherever possible:

```rust
enum HostAction {
    AptInstall { packages: Vec<String> },
    EnableAptComponent { components: Vec<String> },
    AddArchitecture { arch: String },
    CreateUser { user: String, system: bool },
    CreateDirectory { path: PathBuf, owner: String },
    Download { url: Url, dest: PathBuf, sha256: Option<String> },
    RunCommand { program: String, args: Vec<String>, user: Option<String>, cwd: Option<PathBuf> },
    WriteFile { path: PathBuf, contents: String, owner: String, mode: u32 },
    RenderSystemdUnit { name: String, contents: String },
    SystemdDaemonReload,
    SystemdEnable { unit: String },
    SystemdStart { unit: String },
    UfwAllow { port: u16, protocol: Protocol },
}
```

This gives the TUI a dry-run/confirmation screen and keeps root operations auditable.

## Minecraft Adapter

Scope:

- Vanilla, Paper, and Fabric first.
- Modrinth-only mod and modpack browsing/install for MVP.
- Modpack install should use the same `mrpack-install` approach from the Minecraft VPS guide.

Create flow:

1. Ask for server name, folder, Minecraft version, server type, loader/build version when needed, memory, port, online mode, license acceptance, compatible Modrinth options when available, and optional RCON password generation.
2. Install Java based on Minecraft version. Java 21 should be the default for modern 1.20.5+ servers.
3. Create server directory under `/srv/hostable/servers/<id>/`.
4. Install Vanilla, Paper, or Fabric server files.
5. If a Modrinth modpack is selected, download the selected `.mrpack` version and run `mrpack-install` with `--server-dir`.
6. Install any extra individual Modrinth mods and required dependencies.
7. Write `eula.txt`, `server.properties`, and `start.sh`.
8. Enable local RCON by default for safe stop/status and `save-all` before backups. Store the generated password in `/etc/hostable/secrets.env` or an equivalent root-readable secret store.
9. Render `hostable-<id>.service`.
10. Open `25565/tcp` or the chosen port.
11. Start the service and tail logs in the TUI.

Do not use `screen` in managed mode. The blog flow uses `screen` for manual operation, but Hostable should use systemd plus RCON so stop/restart/backups can be automated cleanly. `ExecStop` should send `stop` through local RCON and only fall back to process termination after a timeout.

Config support:

- `server.properties` structured editor.
- Memory settings from `start.sh` or adapter config.
- View/edit allowlist, ops, banned players, and `eula.txt`.
- Raw file editor escape hatch for advanced users.

Modrinth support:

- Search using Modrinth search facets for Minecraft version, loader, and `project_type:mod` or `project_type:modpack`.
- Install Modrinth modpacks through `mrpack-install`, matching the flow in the Minecraft VPS guide.
- Reject projects where server-side support is `unsupported`.
- Fetch project versions through `/project/{id|slug}/version` filtered by loader and game version.
- For individual mods, download the primary file, verify sha512 or sha1 from the API response, and place it in `mods/`.
- Resolve required dependencies where the API response provides them.
- Restart and scan logs for common missing dependency or incompatible mod errors.
- Store a Minecraft mod manifest so Hostable can distinguish base modpack content, user-added mods, and dependencies without backing up all jar files.

Minecraft mod metadata:

```json
{
  "base_modpack": {
    "source": "modrinth",
    "project_id": "...",
    "version_id": "...",
    "file_hash": "..."
  },
  "installed": [
    {
      "source": "modrinth",
      "project_id": "...",
      "version_id": "...",
      "file_name": "...",
      "file_hash": "...",
      "install_reason": "base_modpack"
    },
    {
      "source": "modrinth",
      "project_id": "...",
      "version_id": "...",
      "file_name": "...",
      "file_hash": "...",
      "install_reason": "user_added"
    },
    {
      "source": "modrinth",
      "project_id": "...",
      "version_id": "...",
      "file_name": "...",
      "file_hash": "...",
      "install_reason": "dependency"
    }
  ]
}
```

This should live in the server folder as `hostable.minecraft.mods.json` and be mirrored into SQLite.

Backups:

- Do not stop the server automatically.
- If RCON is available, send `save-all` before copying.
- Include important mutable state: `world/`, `server.properties`, `eula.txt`, `config/`, `defaultconfigs/`, `ops.json`, `whitelist.json`, `banned-players.json`, `banned-ips.json`, and Hostable metadata.
- Do not back up heavy reproducible files by default: server jars, Paper/Fabric installer files, Modrinth caches, or mod jar files.
- Rely on `hostable.minecraft.mods.json` to rehydrate Modrinth modpacks and individual mods later.

Import existing server by folder:

- Detect `server.properties`.
- Detect Vanilla, Paper, or Fabric launch files, `mods/` when present, and `world/`.
- Ask for the desired systemd service name.
- Generate missing Hostable metadata without moving files unless the user opts into migration.

## ARK Adapter

Scope:

- ARK: Survival Evolved first, not ARK: Survival Ascended.
- Fjordur as the default map.
- SteamCMD install.

Create flow:

1. Validate x86_64.
2. Enable Debian `contrib`, `non-free`, and `non-free-firmware` where needed.
3. Add `i386` architecture.
4. Install `steamcmd` and `lib32gcc-s1`.
5. Create `/srv/hostable/servers/<id>/ark-server`.
6. Run SteamCMD with app ID `376030`.
7. Write `start.sh` with map, session name, password, admin password, ports, max players, and optional crossplay.
8. Render `hostable-<id>.service`.
9. Open `7777/udp`, `7778/udp`, `27015/udp`, and optional `27020/tcp` for RCON.
10. Start once to generate `ShooterGame/Saved/Config/LinuxServer/`.

Config support:

- Structured editor for common startup settings:
  - map
  - session name
  - join password
  - admin password
  - max players
  - game port
  - query port
  - RCON port
  - crossplay
- INI editor for:
  - `ShooterGame/Saved/Config/LinuxServer/GameUserSettings.ini`
  - `ShooterGame/Saved/Config/LinuxServer/Game.ini`
- Include curated controls for common values like taming speed, harvest amount, XP multiplier, difficulty, PvE/PvP, and loot quality.
- Always keep a raw text editor fallback because ARK config is large and users will copy settings from wikis.
- No ARK mod management in MVP.

Stop/status:

- Prefer RCON when enabled: save world, broadcast shutdown, then exit.
- Fall back to systemd stop with a conservative timeout.
- Health check should combine `systemctl is-active`, process presence, recent logs, and optionally query/RCON.

Backups:

- Include `ShooterGame/Saved`.
- Do not stop the server automatically.
- Warn that live ARK backups can be less consistent than stopped backups because save files are large and active.

Import existing server by folder:

- Detect `ShooterGame/Binaries/Linux/ShooterGameServer`.
- Detect `ShooterGame/Saved`.
- Read existing configs if present.
- Generate Hostable metadata and systemd service around the existing folder.

## Backup System

Backups should be adapter-driven. Core owns manual backup jobs, archives, retention, uploads, and history. Adapters own the list of important files and any safe pre-copy hooks like Minecraft RCON `save-all`.

Backup flow:

1. Acquire per-server operation lock.
2. Check disk space.
3. Run adapter pre-copy hook if available. For Minecraft, send RCON `save-all` when configured.
4. Create `tar.zst` archive in `/srv/hostable/backups/<server-id>/`.
5. Write manifest JSON with server id, adapter kind, game versions, included paths, archive hash, and timestamp.
6. Upload archive and manifest to S3 if configured.
7. Prune local backups by retention policy.
8. Prune S3 backups by retention policy if enabled.
9. Record success/failure in SQLite and event log.

S3 settings in TUI:

- enabled
- bucket
- region
- endpoint URL for S3-compatible providers
- prefix
- access key env var
- secret key env var
- retention count or age
- test connection button

Use credentials from env/secrets file rather than storing raw secrets in the main config.

## TUI Plan

Use Ratatui with a clear sidebar:

```txt
Servers
  <server list>
  Add server
  Connect folder
Backups
Settings
```

Server detail tabs:

- Overview: status, uptime, ports, path, game version, start/stop/restart.
- Logs: live journal/log tail with search.
- Console: send admin commands through adapter console support, usually RCON.
- Config: structured editor plus raw file editor.
- Mods: Minecraft only for MVP.
- Backups: run backup now, view history, show S3 upload status.
- Danger: remove from Hostable, delete files, regenerate service.

Create-server wizard:

- Choose game: Minecraft or ARK.
- Choose folder and server name.
- Confirm required license acceptance.
- Configure game-specific fields.
- For Minecraft, choose Vanilla/Paper/Fabric, with compatible Modrinth modpack/mod choices only when the selected server type supports them.
- Show dry-run actions.
- Execute install and stream progress.

Interaction model:

- Left sidebar navigation.
- Main pane changes by selected screen.
- Footer shows keybinds.
- Long-running operations run as jobs with progress events.
- Every destructive operation shows an explicit confirmation.
- Every install/import shows a dry-run action list before execution.

Suggested crates:

- `ratatui`
- `crossterm`
- `tokio`
- `color-eyre`
- `serde`
- `toml`
- `sqlx` with SQLite
- `reqwest`
- `tracing`
- `tracing-subscriber`
- `async-trait`
- `directories`
- `clap`

## Admin Web App

The admin UI is intentionally tiny:

- Password login.
- List servers.
- Show status: on, off, starting, stopping, failed, unknown.
- Start button.
- Stop button.
- Minimal event stream for live status changes.

The admin app should be built with Solid Hop and bundled into `hostabled` as static assets. The Rust daemon serves it and owns the API. Keep the web backend in Rust so there is only one supervisor and one source of truth.

API sketch:

```txt
POST /api/login
POST /api/logout
GET  /api/me
GET  /api/servers
GET  /api/servers/:id
POST /api/servers/:id/start
POST /api/servers/:id/stop
GET  /api/events
GET  /api/health
```

Security:

- Store password as Argon2 hash.
- Use HTTP-only SameSite session cookies. Add the `Secure` flag later if Hostable gains first-party HTTPS support.
- Admin web is off by default.
- User must set an admin password before enabling it.
- When enabled, bind directly to `0.0.0.0:8080` and serve the Solid app from `hostabled`.
- Open `8080/tcp` in UFW when the admin web is enabled.
- Do not require nginx or Caddy for MVP.
- Show a warning that MVP admin web is password-gated HTTP, not HTTPS.
- TUI activation message: "You can also add a domain by typing it here, just use this IP for the domain provider: <server-ip>"
- Add CSRF protection for mutation routes.
- Add simple rate limiting on login.

## Landing Page

Use Solid Hop for the landing site. This should be a separate `web/landing` app deployable to Cloudflare.

Landing page sections:

- First viewport: Hostable name, literal value proposition, "Only pay for your VPS, no platform fees", "Free and open-source", and a small one-line install code block.
- Hero tagline: "Server for you and your friends in minutes"
- Hero paragraph: "The cheapest way to run a game server is a VPS. No platform fees, all you pay for is compute, but it's not intuitive. Hostable is a simple CLI that'll make it easy for you to do this without the technical know-how."
- How it works: install, create server, invite friends, manage backups.
- Supported games: Minecraft and ARK now, adapters later.
- No Docker: plain Linux, systemd, real folders.
- Screenshots: TUI server list, Minecraft mods, ARK config, admin web status.
- Product video placeholder: a gray rectangle with a centered play button until the real demo video exists.

Do not make the landing page look like a generic SaaS dashboard. It should feel like a pragmatic self-hosting tool for friends who want a server tonight.

## One-Click Install

Install after ssh'ing into your VPS:

```sh
curl -fsSL https://raw.githubusercontent.com/<owner>/hostable/main/install.sh | sh
```

Installer responsibilities:

- Detect OS, architecture, systemd.
- Download the latest compatible GitHub Release artifact.
- Install `hostable` and `hostabled`.
- Create the `hostable` system user and directories.
- Install `hostabled.service`.
- Start `hostabled`.
- Print `hostable` as the next command.

The TUI should run `doctor` on startup and show missing prerequisites. It can ask for sudo only when it needs to install packages, open firewall ports, or write systemd units.

Release pipeline:

- Use `cargo-dist` for release artifacts and installer-friendly archives.
- Use `/Users/carlo/Desktop/Projects/lazygitrs` as the local reference for the `cargo-dist` profile and GitHub Actions release workflow.
- Keep the install script at a raw GitHub URL, but have it install binaries from GitHub Releases rather than branch artifacts.
- Add custom release notes with the install command, following the `lazygitrs` release workflow pattern.

## Milestones

### Milestone 1: Skeleton and Core

- Convert repo to Rust workspace.
- Add `hostable-core`, `hostabled`, and `hostable-tui` crates.
- Implement registry storage.
- Implement typed action plans and a dry-run renderer.
- Implement fake adapter for tests.
- Add basic TUI shell with sidebar and placeholder screens.

Acceptance:

- `cargo test` passes.
- `hostable` opens a TUI.
- `hostabled` starts and serves `/api/health`.

### Milestone 2: Service Control

- Implement systemd unit rendering.
- Implement start/stop/restart/status.
- Implement operation locks.
- Add job/event stream.
- Add logs view.

Acceptance:

- A dummy managed service can be created, started, stopped, and inspected from TUI and API.

### Milestone 3: Minecraft MVP

- Create Vanilla, Paper, and Fabric Minecraft servers.
- Edit `server.properties`.
- Start/stop through systemd plus local RCON.
- Import existing folder.
- Add Modrinth search and install for modpacks and individual mods.
- Add manual selective local backup.

Acceptance:

- Fresh Minecraft server can be created on a Debian or Ubuntu VPS.
- User can create Vanilla/Paper/Fabric servers, create from a Modrinth modpack where compatible, and add extra server-compatible Modrinth mods.
- Manual backup captures world/config/metadata without archiving reproducible jar files.

### Milestone 4: ARK MVP

- Create ARK: Survival Evolved server with SteamCMD.
- Edit startup settings.
- Edit `GameUserSettings.ini` and `Game.ini`.
- Import existing folder.
- Add manual local backup.

Acceptance:

- Fresh ARK Fjordur server can be created on a Debian or Ubuntu x86_64 VPS.
- Config edits survive restart.
- Manual backup includes `ShooterGame/Saved`.

### Milestone 5: S3 Backups

- Add S3 config UI.
- Add test connection.
- Add upload, manifest, and retention.
- Add backup history screen.

Acceptance:

- Manual backup uploads to S3-compatible storage.
- Retention prunes local and remote backups.

### Milestone 6: Bundled Admin

- Add `web/admin` Solid Hop app.
- Build and bundle admin assets into `hostabled`.
- Implement login/session.
- Implement start/stop/status UI.

Acceptance:

- Browser can log in and start/stop a server through `hostabled`.

### Milestone 7: Landing Page

- Add `web/landing` Solid Hop app.
- Create Cloudflare deployment path.
- Add install CTA and placeholder demo video.

Acceptance:

- Landing page builds independently and can be deployed to Cloudflare.

### Milestone 8: Release

- Add release build pipeline.
- Add install script.
- Add smoke test checklist for Debian and Ubuntu.
- Add manual backup disaster test.

Acceptance:

- New VPS can install Hostable from a GitHub Release with one command and create a Minecraft server without manual shell steps.

## Testing Strategy

- Unit test adapter config parsing and rendering.
- Unit test systemd unit generation.
- Unit test `server.properties` and INI round trips.
- Unit test Modrinth API response parsing with fixtures.
- Integration test action plans with a fake executor.
- Integration test backup archive manifests using temp directories.
- TUI snapshot tests for key screens with `insta` where practical.
- Web tests for login/start/stop flows with Playwright once the admin app exists.
- Manual VPS smoke tests before release because package managers, systemd, ports, and SteamCMD are where real failures happen.

## Important Product Decisions

- Use systemd, not `screen`, for managed servers.
- Use RCON for graceful Minecraft stop and `save-all` before manual backups.
- MVP backups are manually triggered and do not stop servers automatically.
- MVP restore is not first-class; backups must still include enough metadata for future restore.
- Keep game files in real standard folders.
- Keep adapters compiled in until there is a real need for dynamic plugins.
- Keep admin web tiny and password-gated.
- Admin web is off by default; enabling it binds publicly on `0.0.0.0:8080`.
- Use `cargo-dist` and GitHub Releases for distribution.

## Open Questions

- None for the current MVP plan.

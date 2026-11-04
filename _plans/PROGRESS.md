# Hostable Progress

Last updated: 2026-11-03

## Session summary (2026-11-03)

Reviewed with an Opus advisor, then hardened and extended. Verified on a real systemd Debian 12 x86_64 VM (OrbStack), not only in Docker.

### Security and correctness (from the review)

- Daemon only answers loopback unless the web admin is enabled; remote requests can never use the operator token or set the first admin password.
- Operator token `0600 root`; `/var/lib/hostable` and `/etc/hostable` root-owned (game user owns only `/srv/hostable`).
- Input validation: names, install paths under `/srv/hostable/servers`, no control chars, no shell metacharacters in extras, derived service names only, raw register endpoint removed, duplicate names refused.
- Login throttle (5 wrong -> 429 for 60s), 7-day session expiry, constant-time token compare.
- Backups never follow symlinks; `server.properties` and `PalWorldSettings.ini` are `0600`.
- UFW is now actually enabled (SSH allowed first); the daemon port is only opened while the web admin is on.
- RCON client fixed (single-write packets, Source auth quirk, bigger packets). Found by live test: Minecraft dropped the old client, so console and pre-backup saves never worked.
- systemd units: `SuccessExitStatus=143`, `TimeoutStopSec`, `OOMScoreAdjust`; no more `AlwaysPreTouch`; stops no longer end `failed`.

### Features

- **Palworld adapter** (SteamCMD 2394010, UDP 8211): seeds `PalWorldSettings.ini` from the default file, RCON console, backup hook, update plan, `WorldOption.sav` override diagnosis.
- `adapter_for(kind)` replaces the per-game match blocks.
- **Import really works now**: writes a unit running as the folder's owner, keeps the owner's launch script, reads the real port and RCON settings, never auto-enables (no port fight with the old service).
- Port allocator for Minecraft and Palworld (second server gets the next free port); start refuses when a port is already taken.
- Preflight (RAM/swap/disk/running servers), swap creation, disk doctor (+ safe journal/apt cleaning).
- Safe stop/restart/update: refuses while players are online (`--force`), saves first, update backs up first and restarts even if the update failed.
- `hostable <command>` CLI (list, create, import, start, stop, restart, backup, update, console, logs, doctor, disk, swap, preflight).
- Docs site in `docs/` (gittydocs), builds clean with `gittydocs build ./docs`.

### Verified (VM)

- Fresh install via `install.sh`, daemon survives VM restart.
- Minecraft create/start/console/backup/restart/stop through the API and CLI.
- Import of a user-owned folder (runs as that user, RCON picked up).
- Remote lockdown: 403/401/429 matrix from a second machine; web admin enable/disable opens/closes the firewall port.
- Port-conflict refusal, port allocation, preflight, disk report, journal trim, swap no-op.

### Not verified yet

- Palworld/ARK real install and run: SteamCMD is 32-bit x86 and segfaults under Rosetta, so the VM test stopped right after SteamCMD started (apt, user, directories and steamcmd launch worked). Needs a real x86_64 VPS.
- Player-count guard against a live game with players (parsers are unit-tested).
- Admin-web login after the 60s lockout window; real S3 provider; 24h soak.
- GitHub release + `curl | sh` from the real release URL.

### Still open

- Per-server Linux users (all games share `hostable`).
- Job runner with progress streaming (long installs hold one HTTP request open).
- TUI screens for update/doctor/disk (the CLI covers them).
- ARK: one instance per host (fixed ports).
- In-place Minecraft modpack updater.

---

## Earlier status (2026-05-28)

Last updated: 2026-05-28

## Completion

**Agreed MVP scope: 100% implemented.**

**Production confidence: ~92%.**

The MVP is complete against the current plan: TUI, daemon, Minecraft Vanilla/Paper/Fabric, Modrinth support, ARK adapter structure, manual backups, optional admin web UI, landing page, installer, and release packaging are all present and verified. The remaining 8% is not missing planned scope; it is real-world hardening that needs testing on actual VPS providers, larger ARK installs, long-running servers, and user install paths.

## Implemented

- Rust daemon with local API for servers, settings, backups, admin auth, Modrinth, and commands.
- Ratatui TUI with only two top-level screens: `Server` and `Settings`.
- Server opens as a table-first list; create, connect existing folder, server details, logs, config editing, console/admin commands, and install flows are focused subpages instead of side-by-side panels.
- TUI forms use bordered input widgets with visible cursors and searchable list-based select widgets for game/type choices.
- Adapter-based game structure for adding more self-hosted games later.
- Minecraft adapter for Vanilla, Paper, and Fabric.
- Minecraft Modrinth support for search/install selections, modpacks, individual mods, and metadata tracking.
- Minecraft config handling, EULA acceptance handling, startup scripts, systemd unit generation, and backup rules that avoid heavy reproducible mod jars.
- ARK adapter with install plan, SteamCMD flow, ports/firewall, systemd unit, `GameUserSettings.ini`, `Game.ini`, RCON metadata, and config read/write tests.
- Manual backup flow with local rolling retention and optional S3-compatible upload/remote retention.
- Admin web SPA bundled into the Rust-served daemon, gated behind an admin password and disabled unless enabled in settings.
- Admin web supports login/setup, password changes, server list, status polling, start, and stop.
- TUI Settings can set/reset the web admin password with the local operator token, and labels backup retention as local/S3 backup counts instead of terse `keep` fields.
- Landing page copy and install CTA are in place.
- Debian/Ubuntu best-effort installer with service setup, dependency installation, release archive extraction, operator token, admin assets, and config paths.
- `cargo-dist` release pipeline and bundling script for combined `hostable` + `hostabled` + web admin archives.

## Verified

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `bun run build:admin`
- `bun run build:landing`
- `dist build --artifacts=local --output-format=json`
- `sh scripts/bundle-release-artifacts.sh`
- `scripts/smoke-daemon-api.sh`
- Ubuntu installer smoke against bundled x86 Linux archive.
- Debian installer smoke against bundled x86 Linux archive.
- Minecraft live smoke in Ubuntu Docker using the Linux daemon artifact:
  - Vanilla: passed
  - Paper: passed
  - Fabric: passed
- Release archive sanity check:
  - `hostable`
  - `hostabled`
  - `web-admin/index.html`
- `git diff --check`

## Remaining Risk

- ARK is implemented and unit-tested, but not live-smoked end to end because a full ARK/SteamCMD install is large and slow.
- VPS provider firewalls/security groups cannot be fully automated from inside the VPS; Hostable can configure OS firewall rules, but provider-side networking still depends on the host.
- Admin web is intentionally simple and password-gated, but should get more real-world security review before being marketed as hardened.
- Long-running behavior needs soak testing: daemon restarts, systemd restarts, Minecraft memory pressure, backup duration, and server logs over time.
- S3-compatible backups are tested against local HTTP behavior, but should be tested against common real providers before calling it broadly proven.
- The release archives are built locally from macOS via cross build; CI release runs should be validated once pushed.

## Next Hardening Pass

- Live-test ARK on a disposable Ubuntu VPS.
- Test one full install from the real GitHub release URL.
- Test admin web through a public VPS IP with the enabled/disabled settings path.
- Run one real S3 backup against Cloudflare R2, AWS S3, or another S3-compatible provider.
- Do a 24-hour Minecraft soak with one Vanilla/Paper/Fabric server and one manual backup.

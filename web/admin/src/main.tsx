import {
  For,
  Show,
  createEffect,
  createResource,
  createSignal,
  onCleanup,
  untrack,
  type Signal,
} from "solid-js";
import { createStore, reconcile, unwrap } from "solid-js/store";
import { render } from "solid-js/web";
import "./styles.css";
import {
  IconAlert,
  IconGamepad,
  IconKey,
  IconLock,
  IconPlay,
  IconRefresh,
  IconServer,
  IconStop,
  IconUnlock,
} from "./icons";

type ServerRecord = {
  id: string;
  name: string;
  game: "minecraft" | "ark_survival_evolved" | "palworld";
  install_path: string;
  service_name: string;
  ports: Array<{ port: number; protocol: "tcp" | "udp"; label: string }>;
};

type Overview = {
  active: boolean;
  server_type: string;
  game_version: string | null;
  players: { online: number; max: number | null; names: string[] } | null;
  players_note: string | null;
  motd: string | null;
  max_players: number | null;
  online_mode: boolean | null;
  whitelist_enabled: boolean | null;
  whitelist_count: number | null;
  ops_count: number | null;
  mods_count: number | null;
  mods_kind: "mod" | "plugin" | null;
};

// Polling returns fresh objects every time. Reconciling by id keeps the same rows
// in the DOM and only updates what changed, so cards do not flash or reset.
function createServerSignal<T>(initial: T): Signal<T> {
  const [store, setStore] = createStore<{ value: T }>({ value: initial });
  return [
    () => store.value,
    (next: unknown) => {
      const resolved =
        typeof next === "function" ? (next as (prev: T) => T)(untrack(() => unwrap(store.value))) : (next as T);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      setStore("value", reconcile(resolved as any, { key: "id" }));
      return store.value;
    },
  ] as Signal<T>;
}

type ServerView = ServerRecord & {
  status: string;
  overview: Overview | null;
};

type Storage = {
  total_bytes: number;
  used_bytes: number;
  free_bytes: number;
  servers: Array<{ id: string; bytes: number }>;
};

type MeResponse = {
  setup_required: boolean;
  authenticated: boolean;
  admin_web_enabled: boolean;
};

async function fetchMe(): Promise<MeResponse> {
  const response = await fetch("/api/admin/me", { credentials: "include" });
  if (!response.ok) throw new Error("Failed to load auth state");
  return response.json();
}

async function fetchServers(): Promise<ServerRecord[]> {
  const response = await fetch("/api/servers", { credentials: "include" });
  if (!response.ok) throw new Error("Failed to load servers");
  return response.json();
}

async function fetchStorage(authenticated: boolean | undefined): Promise<Storage | null> {
  if (!authenticated) return null;
  try {
    const response = await fetch("/api/system/storage", { credentials: "include" });
    return response.ok ? response.json() : null;
  } catch {
    return null;
  }
}

async function fetchServerStatus(id: string): Promise<string> {
  const response = await fetch(`/api/servers/${id}/status`, { credentials: "include" });
  if (!response.ok) return "unknown";
  const body = await response.json();
  return body.active ?? "unknown";
}

async function fetchOverview(id: string): Promise<Overview | null> {
  try {
    const response = await fetch(`/api/servers/${id}/overview`, { credentials: "include" });
    return response.ok ? response.json() : null;
  } catch {
    return null;
  }
}

async function fetchServerViews(): Promise<ServerView[]> {
  const servers = await fetchServers();
  return Promise.all(
    servers.map(async (server) => {
      const [status, overview] = await Promise.all([
        fetchServerStatus(server.id),
        fetchOverview(server.id),
      ]);
      return { ...server, status, overview };
    }),
  );
}

async function command(id: string, action: "start" | "stop", force = false) {
  const response = await fetch(`/api/servers/${id}/${action}${force ? "?force=true" : ""}`, {
    method: "POST",
    credentials: "include",
  });
  if (response.ok) return;
  const body = await response.json().catch(() => ({}));
  // The daemon refuses to interrupt players unless the caller insists.
  if (response.status === 409 && body.code === "players_online" && !force) {
    const noun = body.players === 1 ? "player is" : "players are";
    if (window.confirm(`${body.players} ${noun} online. ${action === "stop" ? "Stopping" : "Starting"} disconnects them. Continue?`)) {
      return command(id, action, true);
    }
    return;
  }
  throw new Error(body.error ?? `Failed to ${action} server`);
}

async function submitPassword(path: "/api/admin/login" | "/api/admin/setup", password: string) {
  const response = await fetch(path, {
    method: "POST",
    credentials: "include",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ password }),
  });
  if (!response.ok) {
    const body = await response.json().catch(() => ({ error: "request failed" }));
    throw new Error(body.error ?? "request failed");
  }
}

async function changePassword(currentPassword: string, newPassword: string) {
  const response = await fetch("/api/admin/password", {
    method: "PUT",
    credentials: "include",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ current_password: currentPassword, new_password: newPassword }),
  });
  if (!response.ok) {
    const body = await response.json().catch(() => ({ error: "request failed" }));
    throw new Error(body.error ?? "request failed");
  }
}

async function logout() {
  await fetch("/api/admin/logout", { method: "POST", credentials: "include" });
}

function App() {
  const [password, setPassword] = createSignal("");
  const [currentPassword, setCurrentPassword] = createSignal("");
  const [newPassword, setNewPassword] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [passwordMessage, setPasswordMessage] = createSignal<string | null>(null);
  const [lastChecked, setLastChecked] = createSignal<string | null>(null);
  const [me, { refetch: refetchMe }] = createResource(fetchMe);
  const [servers, { refetch }] = createResource(
    () => me()?.authenticated,
    async (authenticated): Promise<ServerView[]> => {
      if (!authenticated) {
        setLastChecked(null);
        return [];
      }
      const views = await fetchServerViews();
      setLastChecked(new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }));
      return views;
    },
    { storage: createServerSignal },
  );
  const [storage, { refetch: refetchStorage }] = createResource(
    () => me()?.authenticated,
    fetchStorage,
  );
  const [busy, setBusy] = createSignal<string | null>(null);
  const serverBytes = (id: string) =>
    storage()?.servers.find((entry) => entry.id === id)?.bytes ?? null;

  const authenticated = () => me()?.authenticated === true;
  const setupRequired = () => me()?.setup_required === true;

  createEffect(() => {
    if (!authenticated()) return;
    const refresh = () => {
      if (document.visibilityState === "visible") {
        void refetch();
        void refetchStorage();
      }
    };
    const interval = window.setInterval(refresh, 5000);
    document.addEventListener("visibilitychange", refresh);
    onCleanup(() => {
      window.clearInterval(interval);
      document.removeEventListener("visibilitychange", refresh);
    });
  });

  const authenticate = async () => {
    setError(null);
    try {
      await submitPassword(setupRequired() ? "/api/admin/setup" : "/api/admin/login", password());
      setPassword("");
      await refetchMe();
      await refetch();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Authentication failed");
    }
  };

  const run = async (id: string, action: "start" | "stop") => {
    setBusy(`${action}:${id}`);
    try {
      await command(id, action);
      await refetch();
    } finally {
      setBusy(null);
    }
  };

  const updatePassword = async () => {
    setPasswordMessage(null);
    try {
      await changePassword(currentPassword(), newPassword());
      setCurrentPassword("");
      setNewPassword("");
      setPasswordMessage("Password updated");
    } catch (caught) {
      setPasswordMessage(caught instanceof Error ? caught.message : "Password update failed");
    }
  };

  return (
    <Show
      when={authenticated()}
      fallback={
        <main class="login-page">
          <form
            class="card login"
            onSubmit={(event) => {
              event.preventDefault();
              authenticate();
            }}
          >
            <img class="mark mark-lg" src="/logo.svg" alt="" width="48" height="44" />
            <div class="login-head">
              <h1>Hostable</h1>
              <p class="muted">Start and stop the game servers on this machine.</p>
            </div>
            <Show
              when={!setupRequired()}
              fallback={
                <p class="notice">
                  No admin password yet. Run <code>sudo hostable</code> on the VPS, open Settings
                  and press <code>p</code> to set one.
                </p>
              }
            >
              <label class="field">
                <span>Password</span>
                <input
                  type="password"
                  autocomplete="current-password"
                  value={password()}
                  onInput={(event) => setPassword(event.currentTarget.value)}
                />
              </label>
              <Show when={error()}>
                <p class="error">{error()}</p>
              </Show>
              <button class="primary wide" type="submit">
                <IconUnlock aria-hidden="true" />
                Unlock
              </button>
            </Show>
          </form>
        </main>
      }
    >
      <header class="bar">
        <div class="bar-inner">
          <span class="brand">
            <img class="mark" src="/logo.svg" alt="" width="26" height="24" />
            Hostable
          </span>
          <div class="bar-actions">
            <Show when={lastChecked()}>
              <span class="muted small">Updated {lastChecked()}</span>
            </Show>
            <button onClick={() => refetch()}>
              <IconRefresh aria-hidden="true" />
              Refresh
            </button>
            <button
              onClick={async () => {
                await logout();
                await refetchMe();
              }}
            >
              <IconLock aria-hidden="true" />
              Lock
            </button>
          </div>
        </div>
      </header>

      <main class="page">
        <div class="page-head">
          <h1>Servers</h1>
          <Show when={servers.state === "ready" || servers.state === "refreshing"}>
            <p class="muted">{summary(servers() ?? [])}</p>
          </Show>
        </div>

        <Show when={storage()?.total_bytes}>
          {(total) => (
            <div class="disk">
              <div
                class="disk-bar"
                role="img"
                aria-label={`${formatBytes(storage()!.used_bytes)} of ${formatBytes(total())} used`}
              >
                <span
                  class={usedPercent(storage()!) >= 85 ? "full" : ""}
                  style={{ width: `${usedPercent(storage()!)}%` }}
                />
              </div>
              <p class="muted small">
                {formatBytes(storage()!.used_bytes)} of {formatBytes(total())} used
                <span aria-hidden="true"> · </span>
                {formatBytes(storage()!.free_bytes)} free
              </p>
            </div>
          )}
        </Show>

        <Show when={servers.error}>
          <p class="notice error-text">
            <IconAlert aria-hidden="true" />
            {servers.error instanceof Error ? servers.error.message : "Failed to load servers"}
          </p>
        </Show>

        <Show
          when={servers.state === "ready" || servers.state === "refreshing" || servers.state === "errored"}
          fallback={<p class="muted">Loading servers...</p>}
        >
          <div class="list">
            <For
              each={servers() ?? []}
              fallback={
                <div class="empty">
                  <span class="tile">
                    <IconServer aria-hidden="true" />
                  </span>
                  <p>No servers yet</p>
                  <p class="muted small">
                    Run <code>sudo hostable</code> on the VPS to create your first one.
                  </p>
                </div>
              }
            >
              {(server) => (
                <div class="item">
                <article class="row">
                  <span class="tile" aria-hidden="true">
                    <GameIcon game={server.game} />
                  </span>
                  <div class="who">
                    <h2>{server.name}</h2>
                    <p class="meta">
                      {gameLabel(server.game)}
                      <span aria-hidden="true"> · </span>
                      <span class="mono">
                        {server.ports.map((port) => `${port.port}/${port.protocol}`).join(", ")}
                      </span>
                      <Show when={playerSummary(server)}>
                        <span aria-hidden="true"> · </span>
                        {playerSummary(server)}
                      </Show>
                      <Show when={serverBytes(server.id) !== null}>
                        <span aria-hidden="true"> · </span>
                        {formatBytes(serverBytes(server.id) ?? 0)}
                      </Show>
                    </p>
                  </div>
                  <p class={`state ${server.status}`} aria-live="polite">
                    <span class="dot" />
                    {statusLabel(server.status)}
                  </p>
                  <div class="actions">
                    <button
                      disabled={busy() === `start:${server.id}` || server.status === "active"}
                      onClick={() => run(server.id, "start")}
                    >
                      <IconPlay aria-hidden="true" />
                      Start
                    </button>
                    <button
                      class="danger"
                      disabled={busy() === `stop:${server.id}` || server.status !== "active"}
                      onClick={() => run(server.id, "stop")}
                    >
                      <IconStop aria-hidden="true" />
                      Stop
                    </button>
                  </div>
                </article>
                <ServerDetails server={server} />
                </div>
              )}
            </For>
          </div>
        </Show>

        <details class="account">
          <summary>
            <IconKey aria-hidden="true" />
            Change password
          </summary>
          <form
            class="password-form"
            onSubmit={(event) => {
              event.preventDefault();
              updatePassword();
            }}
          >
            <label class="field">
              <span>Current password</span>
              <input
                type="password"
                autocomplete="current-password"
                value={currentPassword()}
                onInput={(event) => setCurrentPassword(event.currentTarget.value)}
              />
            </label>
            <label class="field">
              <span>New password</span>
              <input
                type="password"
                autocomplete="new-password"
                value={newPassword()}
                onInput={(event) => setNewPassword(event.currentTarget.value)}
              />
            </label>
            <div class="form-foot">
              <button class="primary" type="submit">
                Update password
              </button>
              <Show when={passwordMessage()}>
                <span class="muted small">{passwordMessage()}</span>
              </Show>
            </div>
          </form>
        </details>
      </main>
    </Show>
  );
}

render(() => <App />, document.getElementById("root")!);

function GameIcon(props: { game: ServerRecord["game"] }) {
  if (props.game === "minecraft") return <img src="/games/minecraft.png" alt="" />;
  if (props.game === "palworld") return <img src="/games/palworld.jpg" alt="" />;
  if (props.game === "ark_survival_evolved") return <img src="/games/ark.jpg" alt="" />;
  return <IconGamepad />;
}

function formatBytes(bytes: number) {
  const gb = bytes / 1024 ** 3;
  if (gb >= 100) return `${Math.round(gb)} GB`;
  if (gb >= 1) return `${gb.toFixed(1)} GB`;
  const mb = bytes / 1024 ** 2;
  if (mb >= 1) return `${Math.round(mb)} MB`;
  return `${Math.max(0, Math.round(bytes / 1024))} KB`;
}

function usedPercent(storage: Storage) {
  if (!storage.total_bytes) return 0;
  return Math.min(100, Math.round((storage.used_bytes / storage.total_bytes) * 100));
}

function summary(servers: ServerView[]) {
  const online = servers.filter((server) => server.status === "active").length;
  const total = `${servers.length} ${servers.length === 1 ? "server" : "servers"}`;
  return `${total}, ${online} online`;
}

function statusLabel(status: string) {
  if (status === "active") return "Online";
  if (status === "inactive") return "Stopped";
  if (status === "failed") return "Failed";
  if (status === "activating") return "Starting";
  if (status === "deactivating") return "Stopping";
  return "Unknown";
}

function gameLabel(game: ServerRecord["game"]) {
  if (game === "minecraft") return "Minecraft";
  if (game === "ark_survival_evolved") return "ARK";
  if (game === "palworld") return "Palworld";
  return game;
}

function playerSummary(server: ServerView) {
  const players = server.overview?.players;
  if (!players) return null;
  const max = players.max ?? server.overview?.max_players;
  return `${players.online}${max ? ` / ${max}` : ""} ${players.online === 1 ? "player" : "players"}`;
}

// The list refetches every few seconds and rebuilds its rows, so remember which
// details are open here rather than in the DOM.
const [openDetails, setOpenDetails] = createSignal<Record<string, boolean>>({});

function ServerDetails(props: { server: ServerView }) {
  const overview = () => props.server.overview;
  const facts = () => {
    const o = overview();
    if (!o) return [];
    const rows: Array<[string, string]> = [];
    if (o.game_version) {
      const loader = o.server_type !== "vanilla" ? ` (${o.server_type})` : "";
      rows.push(["Version", `${o.game_version}${loader}`]);
    }
    if (o.mods_count !== null && o.mods_kind) {
      rows.push([o.mods_kind === "plugin" ? "Plugins" : "Mods", String(o.mods_count)]);
    }
    if (o.whitelist_enabled !== null) {
      const count = o.whitelist_count ?? 0;
      rows.push(["Whitelist", o.whitelist_enabled ? `On, ${count} listed` : "Off, anyone can join"]);
    }
    if (o.ops_count !== null) rows.push(["Operators", String(o.ops_count)]);
    if (o.online_mode !== null) {
      rows.push(["Accounts", o.online_mode ? "Verified by Mojang" : "Offline, names are not verified"]);
    }
    if (o.motd) rows.push(["MOTD", o.motd]);
    return rows;
  };
  return (
    <Show when={overview()}>
      {(o) => (
        <details
          class="detail"
          open={openDetails()[props.server.id] === true}
          onToggle={(event) => {
            const isOpen = event.currentTarget.open;
            setOpenDetails((current) => ({ ...current, [props.server.id]: isOpen }));
          }}
        >
          <summary>Details</summary>
          <div class="detail-body">
            <div class="players">
              <h3>Players online</h3>
              <Show
                when={o().players && o().players!.names.length > 0}
                fallback={
                  <p class="muted small">
                    {o().players ? "Nobody is online." : (o().players_note ?? "Unknown.")}
                  </p>
                }
              >
                <ul class="chips">
                  <For each={o().players!.names}>{(name) => <li>{name}</li>}</For>
                </ul>
              </Show>
            </div>
            <Show when={facts().length > 0}>
              <dl class="facts">
                <For each={facts()}>
                  {([label, value]) => (
                    <>
                      <dt>{label}</dt>
                      <dd>{value}</dd>
                    </>
                  )}
                </For>
              </dl>
            </Show>
            <p class="hint">
              This page is read only. For mods, the whitelist, the live console and server
              settings, run <code>sudo hostable</code> on the VPS to open the terminal UI.
            </p>
          </div>
        </details>
      )}
    </Show>
  );
}

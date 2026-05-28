import { For, Show, createEffect, createResource, createSignal, onCleanup } from "solid-js";
import { render } from "solid-js/web";
import "./styles.css";

type ServerRecord = {
  id: string;
  name: string;
  game: "minecraft" | "ark_survival_evolved";
  install_path: string;
  service_name: string;
  ports: Array<{ port: number; protocol: "tcp" | "udp"; label: string }>;
};

type ServerView = ServerRecord & {
  status: string;
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

async function fetchServerStatus(id: string): Promise<string> {
  const response = await fetch(`/api/servers/${id}/status`, { credentials: "include" });
  if (!response.ok) return "unknown";
  const body = await response.json();
  return body.active ?? "unknown";
}

async function fetchServerViews(): Promise<ServerView[]> {
  const servers = await fetchServers();
  const statuses = await Promise.all(servers.map((server) => fetchServerStatus(server.id)));
  return servers.map((server, index) => ({ ...server, status: statuses[index] }));
}

async function command(id: string, action: "start" | "stop") {
  const response = await fetch(`/api/servers/${id}/${action}`, {
    method: "POST",
    credentials: "include",
  });
  if (!response.ok) throw new Error(`Failed to ${action} server`);
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
    async (authenticated) => {
      if (!authenticated) {
        setLastChecked(null);
        return [];
      }
      const views = await fetchServerViews();
      setLastChecked(new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }));
      return views;
    },
  );
  const [busy, setBusy] = createSignal<string | null>(null);

  const authenticated = () => me()?.authenticated === true;
  const setupRequired = () => me()?.setup_required === true;

  createEffect(() => {
    if (!authenticated()) return;
    const refresh = () => {
      if (document.visibilityState === "visible") void refetch();
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
    <main class="shell">
      <section class="panel auth-panel">
        <div>
          <p class="eyebrow">Hostable Admin</p>
          <h1>Server control, nothing else.</h1>
          <p class="muted">
            Start and stop the game servers already managed by the TUI. This page is protected by
            the daemon, not just the browser.
          </p>
        </div>

        <Show
          when={authenticated()}
          fallback={
            <form
              class="login"
              onSubmit={(event) => {
                event.preventDefault();
                authenticate();
              }}
            >
              <input
                type="password"
                placeholder={setupRequired() ? "Create admin password" : "Admin password"}
                value={password()}
                onInput={(event) => setPassword(event.currentTarget.value)}
              />
              <button type="submit">{setupRequired() ? "Create" : "Unlock"}</button>
              <Show when={error()}>
                <p class="error">{error()}</p>
              </Show>
            </form>
          }
        >
          <div class="authed-stack">
            <div class="toolbar">
              <Show when={lastChecked()}>
                <p class="refresh-meta">Updated {lastChecked()}</p>
              </Show>
              <div class="toolbar-actions">
                <button class="quiet-button" onClick={() => refetch()}>
                  Refresh
                </button>
                <button
                  class="quiet-button"
                  onClick={async () => {
                    await logout();
                    await refetchMe();
                  }}
                >
                  Lock
                </button>
              </div>
            </div>
            <form
              class="password-edit"
              onSubmit={(event) => {
                event.preventDefault();
                updatePassword();
              }}
            >
              <input
                type="password"
                placeholder="Current password"
                value={currentPassword()}
                onInput={(event) => setCurrentPassword(event.currentTarget.value)}
              />
              <input
                type="password"
                placeholder="New password"
                value={newPassword()}
                onInput={(event) => setNewPassword(event.currentTarget.value)}
              />
              <button class="quiet-button" type="submit">
                Change Password
              </button>
              <Show when={passwordMessage()}>
                <p class="refresh-meta">{passwordMessage()}</p>
              </Show>
            </form>
          </div>
        </Show>
      </section>

      <Show when={authenticated()}>
        <section class="server-grid">
          <Show when={servers.error}>
            <div class="panel error-panel">
              {servers.error instanceof Error ? servers.error.message : "Failed to load servers"}
            </div>
          </Show>
          <Show when={!servers.loading} fallback={<div class="panel">Loading servers...</div>}>
            <For each={servers() ?? []} fallback={<div class="panel">No servers registered.</div>}>
              {(server) => (
                <article class="server-card">
                  <div>
                    <p class="eyebrow">{server.game.replaceAll("_", " ")}</p>
                    <h2>{server.name}</h2>
                    <p class="path">{server.install_path}</p>
                    <p class={`status ${server.status}`} aria-live="polite">
                      <span />
                      {statusLabel(server.status)}
                    </p>
                  </div>
                  <dl>
                    <div>
                      <dt>Service</dt>
                      <dd>{server.service_name}</dd>
                    </div>
                    <div>
                      <dt>Ports</dt>
                      <dd>{server.ports.map((port) => `${port.port}/${port.protocol}`).join(", ")}</dd>
                    </div>
                  </dl>
                  <div class="actions">
                    <button
                      disabled={busy() === `start:${server.id}`}
                      onClick={() => run(server.id, "start")}
                    >
                      Start
                    </button>
                    <button
                      class="danger"
                      disabled={busy() === `stop:${server.id}`}
                      onClick={() => run(server.id, "stop")}
                    >
                      Stop
                    </button>
                  </div>
                </article>
              )}
            </For>
          </Show>
        </section>
      </Show>
    </main>
  );
}

render(() => <App />, document.getElementById("root")!);

function statusLabel(status: string) {
  if (status === "active") return "Online";
  if (status === "inactive") return "Stopped";
  if (status === "failed") return "Failed";
  if (status === "activating") return "Starting";
  if (status === "deactivating") return "Stopping";
  return "Unknown";
}

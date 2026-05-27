import { For, Show, createResource, createSignal } from "solid-js";
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

async function fetchServers(): Promise<ServerRecord[]> {
  const response = await fetch("/api/servers");
  if (!response.ok) throw new Error("Failed to load servers");
  return response.json();
}

async function command(id: string, action: "start" | "stop") {
  const response = await fetch(`/api/servers/${id}/${action}`, { method: "POST" });
  if (!response.ok) throw new Error(`Failed to ${action} server`);
}

function App() {
  const [password, setPassword] = createSignal("");
  const [unlocked, setUnlocked] = createSignal(false);
  const [servers, { refetch }] = createResource(fetchServers);
  const [busy, setBusy] = createSignal<string | null>(null);

  const run = async (id: string, action: "start" | "stop") => {
    setBusy(`${action}:${id}`);
    try {
      await command(id, action);
      await refetch();
    } finally {
      setBusy(null);
    }
  };

  return (
    <main class="shell">
      <section class="panel auth-panel">
        <div>
          <p class="eyebrow">Hostable Admin</p>
          <h1>Server control, nothing else.</h1>
          <p class="muted">Start and stop the game servers already managed by the TUI.</p>
        </div>

        <Show
          when={unlocked()}
          fallback={
            <form
              class="login"
              onSubmit={(event) => {
                event.preventDefault();
                if (password().trim().length > 0) setUnlocked(true);
              }}
            >
              <input
                type="password"
                placeholder="Admin password"
                value={password()}
                onInput={(event) => setPassword(event.currentTarget.value)}
              />
              <button type="submit">Unlock</button>
            </form>
          }
        >
          <button class="quiet-button" onClick={() => refetch()}>
            Refresh
          </button>
        </Show>
      </section>

      <Show when={unlocked()}>
        <section class="server-grid">
          <Show when={!servers.loading} fallback={<div class="panel">Loading servers...</div>}>
            <For each={servers() ?? []} fallback={<div class="panel">No servers registered.</div>}>
              {(server) => (
                <article class="server-card">
                  <div>
                    <p class="eyebrow">{server.game.replaceAll("_", " ")}</p>
                    <h2>{server.name}</h2>
                    <p class="path">{server.install_path}</p>
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

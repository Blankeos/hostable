import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import "./styles.css";
import {
  IconCube,
  IconCurrencyDollar,
  IconDownload,
  IconGamepad,
  IconGearSix,
  IconGitBranch,
  IconMinecraft,
  IconPackage,
  IconPaw,
  IconUserPlus,
} from "./assets/icons";

const INSTALL = "curl -fsSL https://raw.githubusercontent.com/blankeos/hostable/main/install.sh | sh";

const STEPS = [
  { icon: IconDownload, title: "Install", body: "Run one command on a fresh Debian or Ubuntu VPS." },
  { icon: IconCube, title: "Create", body: "Pick a game, accept its license, review the plan." },
  { icon: IconUserPlus, title: "Invite", body: "Hostable opens the firewall port and shows the join address." },
  { icon: IconGearSix, title: "Manage", body: "Start, stop, update and back up from the terminal or a web page." },
];

const GAMES = [
  { art: "/games/minecraft.png", icon: IconMinecraft, name: "Minecraft", detail: "Vanilla, Paper, Fabric, Modrinth modpacks" },
  { art: "/games/palworld.jpg", icon: IconPaw, name: "Palworld", detail: "Dedicated server, co-op save import" },
  { art: "/games/ark.jpg", icon: IconGamepad, name: "ARK: Survival Evolved", detail: "Dedicated server through SteamCMD" },
];

function CopyCommand() {
  const [copied, setCopied] = createSignal(false);
  const copy = async () => {
    await navigator.clipboard.writeText(INSTALL);
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  };
  return (
    <div class="command">
      <code>{INSTALL}</code>
      <button type="button" onClick={copy}>
        {copied() ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

function App() {
  return (
    <>
      <header class="nav">
        <div class="wrap nav-inner">
          <span class="brand">
            <img class="mark" src="/logo.svg" alt="" width="26" height="24" />
            Hostable
          </span>
          <nav>
            <a href="https://hostable-docs.carlo.tl">Docs</a>
            <a href="https://github.com/blankeos/hostable">GitHub</a>
          </nav>
        </div>
      </header>

      <main class="wrap">
        <section class="hero">
          <h1>Game servers for you and your friends, in minutes.</h1>
          <p class="lede">
            The cheapest way to run a game server is a VPS: no platform fees, you only pay for
            compute. It is also fiddly. Hostable does the setup for you.
          </p>
          <CopyCommand />
          <p class="hint">Run it after you SSH into your VPS, then type <code>sudo hostable</code>.</p>
          <ul class="facts">
            <li>
              <IconCurrencyDollar aria-hidden="true" />
              Only pay for the VPS
            </li>
            <li>
              <IconGitBranch aria-hidden="true" />
              Free and open source
            </li>
            <li>
              <IconPackage aria-hidden="true" />
              No Docker
            </li>
          </ul>
        </section>

        <section class="block">
          <h2>How it works</h2>
          <ol class="steps">
            {STEPS.map((step) => (
              <li>
                <span class="tile" aria-hidden="true">
                  <step.icon />
                </span>
                <div>
                  <h3>{step.title}</h3>
                  <p>{step.body}</p>
                </div>
              </li>
            ))}
          </ol>
        </section>

        <section class="block">
          <h2>Games</h2>
          <ul class="games">
            {GAMES.map((game) => (
              <li>
                <span class="game-name">
                  <span class="tile" aria-hidden="true">
                    {game.art ? <img src={game.art} alt="" /> : <game.icon />}
                  </span>
                  {game.name}
                </span>
                <span class="detail">{game.detail}</span>
              </li>
            ))}
          </ul>
        </section>
      </main>

      <footer class="wrap foot">
        <span>MIT licensed</span>
        <a href="https://github.com/blankeos/hostable">github.com/blankeos/hostable</a>
      </footer>
    </>
  );
}

render(() => <App />, document.getElementById("root")!);

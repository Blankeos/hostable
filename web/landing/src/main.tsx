import { render } from "solid-js/web";
import { createSignal, onMount, onCleanup } from "solid-js";
import "./styles.css";
import {
  IconDownload,
  IconCube,
  IconUserPlus,
  IconGearSix,
  IconCurrencyDollar,
  IconGitBranch,
  IconPackage,
} from "./assets/icons";

const GAME_NAMES = ["Game", "Minecraft", "ARK"] as const;

function CyclingWord() {
  const [index, setIndex] = createSignal(0);
  const [visible, setVisible] = createSignal(true);

  onMount(() => {
    const interval = setInterval(() => {
      setVisible(false);
      setTimeout(() => {
        setIndex((i) => (i + 1) % GAME_NAMES.length);
        setVisible(true);
      }, 300);
    }, 2500);
    onCleanup(() => clearInterval(interval));
  });

  return (
    <span class="cycle-wrap">
      <span class={`cycle-word${visible() ? " cycle-in" : " cycle-out"}`}>
        {GAME_NAMES[index()]}
      </span>
    </span>
  );
}

function App() {
  return (
    <main>
      <section class="hero">
        <p class="mark">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2" y="2" width="20" height="8" rx="2" />
            <rect x="2" y="14" width="20" height="8" rx="2" />
            <circle cx="6" cy="6" r="1" fill="currentColor" />
            <circle cx="6" cy="18" r="1" fill="currentColor" />
          </svg>
          hostable
        </p>

        <h1>
          <CyclingWord /> server for you<br />
          and your friends in minutes
        </h1>

        <p class="lede">
          The cheapest way to run a game server is a VPS — no platform fees, all you pay for is
          compute. But it&apos;s not intuitive. Hostable handles all of that for you.
        </p>

        <div class="badges">
          <span><IconCurrencyDollar /> Only pay for your VPS</span>
          <span><IconGitBranch /> Free and open-source</span>
          <span><IconPackage /> No Docker required</span>
        </div>

        <div class="install">
          <p>Install after ssh&apos;ing into your VPS</p>
          <code>curl -fsSL https://raw.githubusercontent.com/blankeos/hostable/main/install.sh | sh</code>
        </div>

        <button class="video" aria-label="Play Hostable demo video">
          <span class="play">▶</span>
        </button>
      </section>

      <section class="steps">
        <article>
          <div class="step-icon"><IconDownload /></div>
          <span>Step 1</span>
          <h2>Install</h2>
          <p>Run one command on a fresh Debian or Ubuntu VPS.</p>
        </article>
        <article>
          <div class="step-icon"><IconCube /></div>
          <span>Step 2</span>
          <h2>Create</h2>
          <p>Pick Minecraft or ARK, accept the licenses, and review the action plan.</p>
        </article>
        <article>
          <div class="step-icon"><IconUserPlus /></div>
          <span>Step 3</span>
          <h2>Invite</h2>
          <p>Hostable opens the firewall ports and shows the join address.</p>
        </article>
        <article>
          <div class="step-icon"><IconGearSix /></div>
          <span>Step 4</span>
          <h2>Manage</h2>
          <p>Start, stop, configure, run console commands, and trigger backups from the TUI.</p>
        </article>
      </section>
    </main>
  );
}

render(() => <App />, document.getElementById("root")!);

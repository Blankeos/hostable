import { render } from "solid-js/web";
import "./styles.css";

function App() {
  return (
    <main>
      <section class="hero">
        <div class="copy">
          <p class="mark">hostable</p>
          <h1>Server for you and your friends in minutes</h1>
          <p class="lede">
            The cheapest way to run a game server is a VPS. No platform fees, all you pay for is
            compute, but it&apos;s not intuitive. Hostable is a simple CLI that&apos;ll make it easy
            for you to do this without the technical know-how.
          </p>
          <div class="badges">
            <span>Only pay for your VPS</span>
            <span>Free and open-source</span>
            <span>No Docker required</span>
          </div>
          <div class="install">
            <p>Install after ssh&apos;ing into your VPS</p>
            <code>curl -fsSL https://raw.githubusercontent.com/blankeos/hostable/main/install.sh | sh</code>
          </div>
        </div>

        <button class="video" aria-label="Play Hostable demo video">
          <span class="play">▶</span>
        </button>
      </section>

      <section class="steps">
        <article>
          <span>01</span>
          <h2>Install</h2>
          <p>Run one command on a fresh Debian or Ubuntu VPS.</p>
        </article>
        <article>
          <span>02</span>
          <h2>Create</h2>
          <p>Pick Minecraft or ARK, accept the required licenses, and review the action plan.</p>
        </article>
        <article>
          <span>03</span>
          <h2>Invite</h2>
          <p>Hostable opens the VPS firewall ports and shows the join address.</p>
        </article>
        <article>
          <span>04</span>
          <h2>Manage</h2>
          <p>Start, stop, configure, run console commands, and trigger backups from the TUI.</p>
        </article>
      </section>
    </main>
  );
}

render(() => <App />, document.getElementById("root")!);

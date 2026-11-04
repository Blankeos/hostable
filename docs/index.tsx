import { createSignal } from "solid-js"
import { withBasePath } from "@/utils/base-path"
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
} from "./_components/icons"
import "./_components/landing.css"
import { TuiDemo } from "./_components/tui-demo"

export const sidebar = false
export const toc = false
export const title = "Hostable"
export const description =
  "Run Minecraft, Palworld or ARK servers on your VPS with one command. No platform fees, no Docker."

const INSTALL = "curl -fsSL https://raw.githubusercontent.com/blankeos/hostable/main/install.sh | sh"

const STEPS = [
  { icon: IconDownload, title: "Install", body: "Run one command on a fresh Debian or Ubuntu VPS." },
  { icon: IconCube, title: "Create", body: "Pick a game, accept its license, review the plan." },
  { icon: IconUserPlus, title: "Invite", body: "Hostable opens the firewall port and shows the join address." },
  { icon: IconGearSix, title: "Manage", body: "Start, stop, update and back up from the terminal or a web page." },
]

const GAMES = [
  { art: "/static/games/minecraft.png", icon: IconMinecraft, name: "Minecraft", detail: "Vanilla, Paper, Fabric, Modrinth modpacks" },
  { art: "/static/games/palworld.jpg", icon: IconPaw, name: "Palworld", detail: "Dedicated server, co-op save import" },
  { art: "/static/games/ark.jpg", icon: IconGamepad, name: "ARK: Survival Evolved", detail: "Dedicated server through SteamCMD" },
]

function CopyCommand() {
  const [copied, setCopied] = createSignal(false)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(INSTALL)
      setCopied(true)
      setTimeout(() => setCopied(false), 1600)
    } catch {
      // clipboard blocked: the command stays selectable
    }
  }
  return (
    <div class="hl-command">
      <code>{INSTALL}</code>
      <button type="button" onClick={copy}>
        {copied() ? "Copied" : "Copy"}
      </button>
    </div>
  )
}

export default function Page() {
  return (
    <div class="hl">
      <main class="hl-wrap">
        <section class="hl-hero">
          <h1>Game servers for you and your friends, in minutes.</h1>
          <p class="hl-lede">
            The cheapest way to run a game server is a VPS: no platform fees, you only pay for
            compute. It is also fiddly. Hostable does the setup for you.
          </p>
          <CopyCommand />
          <p class="hl-hint">
            Run it after you SSH into your VPS, then type <code>sudo hostable</code>. <a href={withBasePath("/introduction")}><u>Read the docs</u></a>.
          </p>
          <ul class="hl-facts">
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

        <section class="hl-block" aria-label="Hostable terminal app demo">
          <div class="hl-head">
            <h2>In your terminal</h2>
            <p class="hl-hint">The real app, captured as text and replayed.</p>
          </div>
          <TuiDemo />
        </section>

        <section class="hl-block">
          <h2>How it works</h2>
          <ol class="hl-steps">
            {STEPS.map((step) => (
              <li>
                <span class="hl-tile" aria-hidden="true">
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

        <section class="hl-block">
          <h2>Games</h2>
          <ul class="hl-games">
            {GAMES.map((game) => (
              <li>
                <span class="hl-game-name">
                  <span class="hl-tile" aria-hidden="true">
                    <img src={withBasePath(game.art)} alt="" />
                  </span>
                  {game.name}
                </span>
                <span class="hl-detail">{game.detail}</span>
              </li>
            ))}
          </ul>
        </section>
      </main>

      <footer class="hl-wrap hl-foot">
        <span>MIT licensed</span>
        <a href="https://github.com/blankeos/hostable">github.com/blankeos/hostable</a>
      </footer>
    </div>
  )
}

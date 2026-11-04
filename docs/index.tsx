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
  { icon: IconDownload, title: "Install" },
  { icon: IconCube, title: "Create" },
  { icon: IconUserPlus, title: "Invite" },
  { icon: IconGearSix, title: "Manage" },
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
          <p class="hl-lede">Hostable sets up game servers on your own VPS.</p>
          <CopyCommand />
          <p class="hl-hint">
            Run on your VPS, then <code>sudo hostable</code>. <a href={withBasePath("/introduction")}><u>Docs</u></a>
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
          <h2>In your terminal</h2>
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
                <h3>{step.title}</h3>
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

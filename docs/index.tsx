import { createSignal, onCleanup, onMount } from "solid-js"
import { withBasePath } from "@/utils/base-path"
import {
  IconArrowRight,
  IconCube,
  IconCurrencyDollar,
  IconDownload,
  IconGearSix,
  IconGitBranch,
  IconPackage,
  IconUserPlus,
} from "./_components/icons"
import "./_components/landing.css"
import { TuiDemo } from "./_components/tui-demo"
import { Ground, VoxelBlock, VoxelIsland } from "./_components/voxel"

export const sidebar = false
export const toc = false
export const title = "Hostable"
export const description =
  "Run Minecraft, Palworld or ARK servers on your VPS with one command. No platform fees, no Docker."

const INSTALL = "curl -fsSL https://raw.githubusercontent.com/blankeos/hostable/main/install.sh | sh"
const RUN = "sudo hostable"

// Typing timing in ms, handed to the CSS through custom properties.
const START = 700
const STEP_INSTALL = 16
const STEP_RUN = 60
const START_RUN = START + (INSTALL.length + 1) * STEP_INSTALL + 400

const STEPS = [
  { icon: IconDownload, title: "Install" },
  { icon: IconCube, title: "Create" },
  { icon: IconUserPlus, title: "Invite" },
  { icon: IconGearSix, title: "Manage" },
]

const GAMES = [
  { art: "/static/games/minecraft.png", name: "Minecraft", detail: "Vanilla, Paper, Fabric, Modrinth modpacks" },
  { art: "/static/games/palworld.jpg", name: "Palworld", detail: "Dedicated server, co-op save import" },
  { art: "/static/games/ark.jpg", name: "ARK: Survival Evolved", detail: "Dedicated server through SteamCMD" },
]

function CopyButton(props: { text: string; label: string }) {
  const [copied, setCopied] = createSignal(false)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(props.text)
      setCopied(true)
      setTimeout(() => setCopied(false), 1600)
    } catch {
      // clipboard blocked: the command stays selectable
    }
  }
  return (
    <button type="button" class="hl-copy" aria-label={props.label} onClick={copy}>
      {copied() ? "Copied" : "Copy"}
    </button>
  )
}

// Typed out with CSS only (one span per character): it animates before hydration
// and the full command stays selectable the whole time.
function Typed(props: { text: string; start: number; step: number; caret?: boolean }) {
  return (
    <code style={{ "--t0": `${props.start}ms`, "--step": `${props.step}ms` }}>
      <span class="hl-prompt" aria-hidden="true">
        $
      </span>
      {props.text.split("").map((ch, i) => (
        <span class="hl-ch" style={{ "--i": i }}>
          {ch}
        </span>
      ))}
      {props.caret && <span class="hl-caret" style={{ "--i": props.text.length }} aria-hidden="true" />}
    </code>
  )
}

function Install() {
  return (
    <ol class="hl-install">
      <li>
        <h2>
          <span class="hl-num" aria-hidden="true">
            1
          </span>
          Install
          <span class="hl-where">on your VPS</span>
        </h2>
        <div class="hl-cmd">
          <Typed text={INSTALL} start={START} step={STEP_INSTALL} />
          <CopyButton text={INSTALL} label="Copy install command" />
        </div>
      </li>
      <li>
        <h2>
          <span class="hl-num" aria-hidden="true">
            2
          </span>
          Run
        </h2>
        <div class="hl-cmd">
          <Typed text={RUN} start={START_RUN} step={STEP_RUN} caret />
          <CopyButton text={RUN} label="Copy run command" />
        </div>
      </li>
    </ol>
  )
}

export default function Page() {
  let root: HTMLDivElement | undefined

  onMount(() => {
    // Reveal sections as they scroll in. Without JS everything is simply visible.
    root!.classList.add("hl-js")
    const seen = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue
          entry.target.classList.add("is-in")
          seen.unobserve(entry.target)
        }
      },
      { rootMargin: "0px 0px -10% 0px" },
    )
    root!.querySelectorAll(".hl-reveal").forEach((el) => seen.observe(el))
    onCleanup(() => seen.disconnect())
  })

  return (
    <div class="hl" ref={root}>
      <main>
        <section class="hl-hero">
          <div class="hl-floaters" aria-hidden="true">
            <VoxelBlock kind="grass" class="hl-float hl-float-1" />
            <VoxelBlock kind="server" class="hl-float hl-float-2" />
            <VoxelBlock kind="stone" class="hl-float hl-float-3" />
            <VoxelBlock kind="leaves" class="hl-float hl-float-4" />
          </div>
          <VoxelIsland class="hl-island" />
          <h1>Game servers for you and your friends, in minutes.</h1>
          <p class="hl-lede">Hostable sets up game servers on your own VPS.</p>
          <Install />
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
            <li>
              <a href={withBasePath("/introduction")} class="hl-docs">
                Docs
                <IconArrowRight aria-hidden="true" />
              </a>
            </li>
          </ul>
        </section>

        <section class="hl-wrap hl-block hl-reveal" aria-label="Hostable terminal app demo">
          <h2>In your terminal</h2>
          <TuiDemo />
        </section>

        <section class="hl-wrap hl-block hl-reveal">
          <h2>How it works</h2>
          <ol class="hl-steps">
            {STEPS.map((step, i) => (
              <li style={{ "--i": i }}>
                <span class="hl-block-tile" aria-hidden="true">
                  <step.icon />
                </span>
                <h3>{step.title}</h3>
              </li>
            ))}
          </ol>
        </section>

        <section class="hl-wrap hl-block hl-reveal">
          <h2>Games</h2>
          <ul class="hl-games">
            {GAMES.map((game, i) => (
              <li style={{ "--i": i }}>
                <img src={withBasePath(game.art)} alt="" />
                <h3>{game.name}</h3>
                <p>{game.detail}</p>
              </li>
            ))}
          </ul>
        </section>
      </main>

      <footer class="hl-foot">
        <div class="hl-wrap hl-foot-row">
          <span>MIT licensed</span>
          <a href="https://github.com/blankeos/hostable">github.com/blankeos/hostable</a>
        </div>
        <Ground />
      </footer>
    </div>
  )
}

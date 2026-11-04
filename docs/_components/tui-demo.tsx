import { createMemo, createSignal, For, onCleanup, onMount } from "solid-js"
import { CHAPTERS, COLS, FRAMES, LINES, ROWS, STYLES } from "./tui-demo-frames"

// Frames worth stopping on when stepping (skips the per-keystroke typing frames).
const KEY_FRAMES = FRAMES.map((f, i) => (f[1] >= 500 ? i : -1)).filter((i) => i >= 0)
const CHAPTER_START = CHAPTERS.map((_, c) => FRAMES.findIndex((f) => f[0] === c))
// Shown for reduced motion: the last Create frame, the form with the modpack and the extra mod.
const STILL = CHAPTER_START[2] - 1
const CHAPTER_MS = CHAPTERS.map((_, c) => FRAMES.reduce((sum, f) => sum + (f[0] === c ? f[1] : 0), 0))

const STYLE_CLASS = STYLES.map(([fg, bg, flags]) =>
  [fg && `t-${fg}`, bg && `t-bg-${bg}`, ...flags.split("").map((f) => `t-${f}`)].filter(Boolean).join(" "),
)

// Glyphs outside ASCII and box drawing may come from a fallback font with another
// width; pin them to one cell so the columns stay aligned.
const WIDE = /([^\x00-\x7f─-▟])/

function Line(props: { runs: (string | number)[] }) {
  const parts: { cls: string; text: string; cell?: boolean }[] = []
  for (let i = 0; i < props.runs.length; i += 2) {
    const cls = STYLE_CLASS[props.runs[i] as number]
    for (const piece of (props.runs[i + 1] as string).split(WIDE)) {
      if (piece) parts.push({ cls, text: piece, cell: WIDE.test(piece) })
    }
  }
  return (
    <div class="hl-term-line">
      {parts.map((p) => (
        <span class={(p.cell ? `${p.cls} hl-term-cell` : p.cls) || undefined}>{p.text}</span>
      ))}
    </div>
  )
}

export function TuiDemo() {
  const [frame, setFrame] = createSignal(0)
  const [elapsed, setElapsed] = createSignal(0)
  const [playing, setPlaying] = createSignal(false)
  let visible = false
  let raf = 0
  let last = 0
  let screen: HTMLDivElement | undefined
  let viewport: HTMLDivElement | undefined

  const current = () => FRAMES[frame()]
  const lines = createMemo(() => current()[3].map((i) => LINES[i]))
  const progress = (c: number) => {
    const [chapter] = current()
    if (c < chapter) return 1
    if (c > chapter) return 0
    let done = elapsed()
    for (let i = CHAPTER_START[c]; i < frame(); i++) done += FRAMES[i][1]
    return Math.min(1, done / CHAPTER_MS[c])
  }

  const tick = (now: number) => {
    let t = elapsed() + Math.min(now - last, 100)
    last = now
    let f = frame()
    while (t >= FRAMES[f][1]) {
      t -= FRAMES[f][1]
      f = (f + 1) % FRAMES.length
    }
    setFrame(f)
    setElapsed(t)
    raf = requestAnimationFrame(tick)
  }
  const run = () => {
    cancelAnimationFrame(raf)
    if (playing() && visible && !document.hidden) {
      last = performance.now()
      raf = requestAnimationFrame(tick)
    }
  }
  const go = (f: number) => {
    setFrame((f + FRAMES.length) % FRAMES.length)
    setElapsed(0)
  }
  const step = (dir: 1 | -1) => {
    setPlaying(false)
    run()
    const f = frame()
    const next = dir > 0 ? KEY_FRAMES.find((k) => k > f) : [...KEY_FRAMES].reverse().find((k) => k < f)
    go(next ?? (dir > 0 ? KEY_FRAMES[0] : KEY_FRAMES[KEY_FRAMES.length - 1]))
  }
  const toggle = () => {
    setPlaying(!playing())
    run()
  }
  const jump = (c: number) => {
    go(CHAPTER_START[c])
    run()
  }

  // Fit 108 columns to the frame width: measure the grid once at a known size.
  const fit = () => {
    if (!screen || !viewport) return
    screen.style.fontSize = "10px"
    const perPx = screen.scrollWidth / 10
    const pad = getComputedStyle(viewport)
    const room = viewport.clientWidth - parseFloat(pad.paddingLeft) - parseFloat(pad.paddingRight)
    const size = Math.min(13, Math.max(4, room / perPx))
    screen.style.fontSize = `${Math.floor(size * 100) / 100}px`
  }

  onMount(() => {
    fit()
    const resize = new ResizeObserver(fit)
    resize.observe(viewport!)
    const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches
    if (still) go(STILL)
    else setPlaying(true)
    const seen = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting
      run()
    })
    seen.observe(viewport!)
    document.addEventListener("visibilitychange", run)
    onCleanup(() => {
      cancelAnimationFrame(raf)
      resize.disconnect()
      seen.disconnect()
      document.removeEventListener("visibilitychange", run)
    })
  })

  return (
    <div class="hl-demo">
      <div class="hl-term">
        <div class="hl-term-bar" aria-hidden="true">
          <span />
          <span />
          <span />
        </div>
        <div class="hl-term-view" ref={viewport}>
          <div class="hl-term-screen" ref={screen} style={{ width: `${COLS}ch`, height: `${ROWS * 1.2}em` }}>
            <For each={lines()}>{(runs) => <Line runs={runs} />}</For>
            {current()[2] && (
              <span
                class="hl-term-cursor"
                aria-hidden="true"
                style={{ left: `${current()[2]![0]}ch`, top: `${current()[2]![1] * 1.2}em` }}
              />
            )}
          </div>
        </div>
      </div>
      <div class="hl-demo-controls">
        <div class="hl-demo-steps">
          {CHAPTERS.map((name, c) => (
            <button type="button" aria-label={`Jump to ${name}`} onClick={() => jump(c)}>
              <span style={{ transform: `scaleX(${progress(c)})` }} />
            </button>
          ))}
        </div>
        <button type="button" class="hl-demo-icon" aria-label="Previous step" onClick={() => step(-1)}>
          <svg viewBox="0 0 16 16" aria-hidden="true">
            <path d="M10 3.5 5.5 8l4.5 4.5" />
          </svg>
        </button>
        <button type="button" class="hl-demo-icon" aria-label={playing() ? "Pause" : "Play"} onClick={toggle}>
          <svg viewBox="0 0 16 16" aria-hidden="true">
            {playing() ? <path d="M5.5 3.5v9M10.5 3.5v9" /> : <path d="M5 3.2v9.6L12.5 8Z" class="fill" />}
          </svg>
        </button>
        <button type="button" class="hl-demo-icon" aria-label="Next step" onClick={() => step(1)}>
          <svg viewBox="0 0 16 16" aria-hidden="true">
            <path d="M6 3.5 10.5 8 6 12.5" />
          </svg>
        </button>
      </div>
    </div>
  )
}

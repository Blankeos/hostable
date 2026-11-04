// Hand-drawn isometric voxel art for the landing page. Every block is flat-coloured
// polygons generated here; no textures or third-party assets.

type Kind = "grass" | "dirt" | "stone" | "wood" | "leaves" | "server" | "buddy"
type Cube = [x: number, y: number, z: number, kind: Kind]
type Face = "top" | "left" | "right"

const W = 40 // tile width
const H = 20 // top diamond height
const S = 22 // side height
const N = 4 // pixels per face edge

// Flat palettes per face. Index 0 is the base colour, the rest are pixel variations.
const PAL: Record<string, string[]> = {
  "grass-top": ["#6aa84f", "#5f9c45", "#77b65a", "#5a9441"],
  "grass-left": ["#4f8a39", "#467e33"],
  "grass-right": ["#3f7230", "#38682b"],
  "dirt-left": ["#8a5a3b", "#7b4f33", "#946243", "#6e452c"],
  "dirt-right": ["#6f4630", "#633f2a", "#7a4e36", "#583824"],
  "dirt-top": ["#946243", "#8a5a3b", "#a06c4b"],
  "stone-top": ["#a3a3a3", "#989898", "#adadad"],
  "stone-left": ["#8a8a8a", "#7e7e7e", "#939393", "#747474"],
  "stone-right": ["#707070", "#666666", "#787878", "#5d5d5d"],
  "wood-top": ["#b08455", "#9e7548"],
  "wood-left": ["#7a5534", "#6c4a2d", "#85603b"],
  "wood-right": ["#5f4128", "#553a24", "#694a2e"],
  "leaves-top": ["#4ca146", "#3f8f3a", "#58ad50", "#368034"],
  "leaves-left": ["#3b8436", "#33752f", "#43913d"],
  "leaves-right": ["#2f6a2b", "#285d25", "#367530"],
  "server-top": ["#d4582a", "#c4481a", "#dc6532"],
  "server-left": ["#c4481a", "#b94418"],
  "server-right": ["#a33a14", "#983612"],
  "buddy-top": ["#86c95f", "#7cbf56", "#90d168"],
  "buddy-left": ["#6fb24c", "#67a846"],
  "buddy-right": ["#5a983c", "#538e37"],
}

// Deterministic noise so server and client render the same art.
function noise(a: number, b: number, c: number) {
  let h = (a * 374761393 + b * 668265263 + c * 2147483647) | 0
  h = Math.imul(h ^ (h >>> 13), 1274126177)
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296
}

function pick(list: string[], r: number) {
  // Mostly the base colour, sometimes a variation: reads as texture, not static.
  return r < 0.45 ? list[0] : list[1 + Math.floor(((r - 0.45) / 0.55) * (list.length - 1))]
}

type Pixel = { points: string; fill: string; cls?: string }

function facePixels(kind: Kind, face: Face, X: number, Y: number, seed: number): Pixel[] {
  // Corner + two edge vectors spanning the face.
  const [ox, oy, ux, uy, vx, vy] =
    face === "top"
      ? [X, Y - H / 2, W / 2, H / 2, -W / 2, H / 2]
      : face === "left"
        ? [X - W / 2, Y, W / 2, H / 2, 0, S]
        : [X, Y + H / 2, W / 2, -H / 2, 0, S]
  const at = (u: number, v: number) => `${(ox + ux * u + vx * v).toFixed(2)},${(oy + uy * u + vy * v).toFixed(2)}`
  const out: Pixel[] = []
  for (let r = 0; r < N; r++) {
    for (let c = 0; c < N; c++) {
      const n = noise(seed, r * N + c, face.length)
      let fill: string
      let cls: string | undefined
      if (kind === "grass" && face !== "top") {
        // Grass hangs over the dirt sides by a pixel or two.
        const lip = r === 0 || (r === 1 && n < 0.35)
        fill = pick(PAL[lip ? `grass-${face}` : `dirt-${face}`], n)
      } else if (kind === "server" && face !== "top") {
        const bay = (r === 1 || r === 3) && c < 3
        fill = bay ? "#7a2a0d" : pick(PAL[`server-${face}`], n)
        if (r === 1 && c === 3) {
          fill = "#7ee08f"
          cls = "hl-vx-led"
        } else if (r === 3 && c === 3) fill = "#f2c14e"
      } else if (kind === "buddy" && face === "right") {
        const eye = r === 1 && (c === 0 || c === 3)
        const mouth = r === 3 && (c === 1 || c === 2)
        fill = eye || mouth ? "#1d2a17" : pick(PAL["buddy-right"], n)
        if (eye) cls = "hl-vx-eye"
      } else if (kind === "wood" && face === "top") {
        fill = (r === 0 || r === 3 || c === 0 || c === 3) ? PAL["wood-top"][1] : PAL["wood-top"][0]
      } else {
        fill = pick(PAL[`${kind}-${face}`], n)
      }
      const u0 = c / N
      const u1 = (c + 1) / N
      const v0 = r / N
      const v1 = (r + 1) / N
      out.push({ points: `${at(u0, v0)} ${at(u1, v0)} ${at(u1, v1)} ${at(u0, v1)}`, fill, cls })
    }
  }
  return out
}

function project(x: number, y: number, z: number) {
  return [(x - y) * (W / 2), (x + y) * (H / 2) - z * S] as const
}

function bounds(cubes: Cube[], pad: number) {
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  for (const [x, y, z] of cubes) {
    const [X, Y] = project(x, y, z)
    minX = Math.min(minX, X - W / 2)
    maxX = Math.max(maxX, X + W / 2)
    minY = Math.min(minY, Y - H / 2)
    maxY = Math.max(maxY, Y + H / 2 + S)
  }
  return `${minX - pad} ${minY - pad} ${maxX - minX + pad * 2} ${maxY - minY + pad * 2}`
}

function CubeShape(props: { cube: Cube; order: number }) {
  const [x, y, z, kind] = props.cube
  const [X, Y] = project(x, y, z)
  const seed = x * 31 + y * 17 + z * 7 + 3
  const pixels = [
    ...facePixels(kind, "top", X, Y, seed),
    ...facePixels(kind, "left", X, Y, seed + 1),
    ...facePixels(kind, "right", X, Y, seed + 2),
  ]
  return (
    <g class="hl-vx-cube" style={{ "--i": props.order }}>
      {pixels.map((p) => (
        <polygon points={p.points} fill={p.fill} class={p.cls} />
      ))}
    </g>
  )
}

function Voxels(props: { cubes: Cube[]; class?: string; pad?: number }) {
  // Paint back to front; animate bottom to top so the island builds up.
  const drawn = [...props.cubes].sort((a, b) => a[0] + a[1] + a[2] - (b[0] + b[1] + b[2]) || a[2] - b[2])
  const build = [...props.cubes].sort((a, b) => a[2] - b[2] || a[0] + a[1] - (b[0] + b[1]))
  return (
    <svg
      class={props.class}
      viewBox={bounds(props.cubes, props.pad ?? 2)}
      aria-hidden="true"
      shape-rendering="crispEdges"
    >
      {drawn.map((cube) => (
        <CubeShape cube={cube} order={build.indexOf(cube)} />
      ))}
    </svg>
  )
}

const range = (n: number) => Array.from({ length: n }, (_, i) => i)
const layer = (z: number, kind: Kind, keep: (x: number, y: number) => boolean): Cube[] =>
  range(4).flatMap((x) => range(4).filter((y) => keep(x, y)).map((y) => [x, y, z, kind] as Cube))

const ISLAND: Cube[] = [
  // underside, tapering down
  ...layer(-2, "stone", (x, y) => x > 0 && y > 0 && x + y >= 3 && x + y <= 5),
  ...layer(-1, "dirt", (x, y) => x + y >= 1 && x + y <= 5 && !(x === 3 && y === 0) && !(x === 0 && y === 3)),
  // grass top
  ...layer(0, "grass", () => true),
  // a tree at the back
  [0, 0, 1, "wood"],
  [0, 0, 2, "leaves"],
  [1, 0, 2, "leaves"],
  [0, 1, 2, "leaves"],
  [0, 0, 3, "leaves"],
  // the server, a spare block, and a friend
  [2, 1, 1, "server"],
  [2, 1, 2, "server"],
  [3, 0, 1, "stone"],
  [1, 3, 1, "buddy"],
]

export function VoxelIsland(props: { class?: string }) {
  return <Voxels cubes={ISLAND} class={props.class} pad={4} />
}

export function VoxelBlock(props: { kind: Kind; class?: string }) {
  return <Voxels cubes={[[0, 0, 0, props.kind]]} class={props.class} />
}

// A strip of pixel grass and dirt for the bottom of the page: one 64x32 tile, repeated.
const GROUND_PX = 4
const GROUND_COLS = 16
const GROUND_ROWS = 8
const GROUND = range(GROUND_COLS).flatMap((c) => {
  const drip = noise(c, 7, 1) < 0.3 ? 3 : noise(c, 8, 2) < 0.6 ? 2 : 1
  return range(GROUND_ROWS).map((r) => {
    const n = noise(c, r, 5)
    const kind = r <= drip ? "grass-top" : r === drip + 1 && n < 0.2 ? "grass-left" : "dirt-left"
    return { x: c * GROUND_PX, y: r * GROUND_PX, fill: pick(PAL[kind], n) }
  })
})

export function Ground() {
  const w = GROUND_COLS * GROUND_PX
  const h = GROUND_ROWS * GROUND_PX
  return (
    <svg class="hl-ground" width="100%" height={h} aria-hidden="true" shape-rendering="crispEdges">
      <defs>
        <pattern id="hl-ground-tile" width={w} height={h} patternUnits="userSpaceOnUse">
          {GROUND.map((p) => (
            <rect x={p.x} y={p.y} width={GROUND_PX} height={GROUND_PX} fill={p.fill} />
          ))}
        </pattern>
      </defs>
      <rect width="100%" height={h} fill="url(#hl-ground-tile)" />
    </svg>
  )
}

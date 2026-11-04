#!/usr/bin/env python3
"""Capture the landing-page TUI demo frames from the real `hostable` binary.

Regenerate after the TUI changes:

    cargo build -p hostable
    python3 -m venv /tmp/pyte-venv && /tmp/pyte-venv/bin/pip install pyte
    /tmp/pyte-venv/bin/python scripts/capture-tui-demo.py            # writes docs/_components/tui-demo-frames.ts
    /tmp/pyte-venv/bin/python scripts/capture-tui-demo.py --text     # print the frames as plain text instead

Needs: macOS or Linux (uses a pty), Python 3.9+ with `pyte`, and target/debug/hostable
(override with --bin). It starts a small canned daemon on 127.0.0.1 (no network, no VPS),
runs the TUI in a 108x30 pty, feeds the keys in SCRIPT one step at a time and snapshots
the screen after each step as rows of styled text runs.
"""
import argparse, fcntl, http.server, json, os, pty, select, socket, struct, sys, tempfile
import termios, threading, time

import pyte

COLS, ROWS = 108, 30
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# ---------------------------------------------------------------- canned daemon

SERVERS = [
    {"id": "a", "name": "Cobbleverse", "game": "minecraft", "install_path": "/srv/a", "service_name": "hostable-a.service", "adapter_config": {}, "ports": [{"port": 25565, "protocol": "tcp", "label": "game"}]},
    {"id": "b", "name": "Palworld Friends", "game": "palworld", "install_path": "/srv/b", "service_name": "hostable-b.service", "adapter_config": {}, "ports": [{"port": 8211, "protocol": "udp", "label": "game"}]},
    {"id": "c", "name": "ARK Island", "game": "ark_survival_evolved", "install_path": "/srv/c", "service_name": "hostable-c.service", "adapter_config": {}, "ports": [{"port": 7777, "protocol": "udp", "label": "game"}]},
]
STATUS = {"a": "active", "b": "inactive", "c": "failed"}
WL = ["Blankeos", "Talaba"]
OPS = [("Blankeos", 4)]
ONLINE = ["Blankeos", "Talaba", "kuya_migs"]
MODS = [("Cobblemon", "cobblemon", "1.6.1", "Cobblemon-fabric-1.6.1.jar", True, 98.2e6),
        ("Fabric API", "fabric-api", "0.105.0", "fabric-api-0.105.0.jar", True, 2.2e6),
        ("Lithium", "lithium", "0.13.1", "lithium-fabric-0.13.1.jar", True, 2.6e6),
        ("FerriteCore", "ferrite-core", "7.0.0", "ferritecore-7.0.0.jar", True, 0.2e6),
        ("Simple Voice Chat", "simple-voice-chat", "2.5.24", "voicechat-fabric-2.5.24.jar", True, 1.9e6),
        ("Chunky", "chunky", "1.4.16", "chunky-fabric-1.4.16.jar", True, 0.3e6),
        ("SkinRestorer", "skinrestorer", "2.1.0", "skinrestorer-2.1.0.jar", True, 1.4e6),
        ("ModernFix", "modernfix", "5.20.0", "modernfix-fabric-5.20.0.jar", False, 0.9e6)]
LOG = "2026-11-04T{t}+0800 vps hostable-a[991]: [{t}] [Server thread/{lvl}]: {msg}"
LOGS = [LOG.format(t=t, lvl=lvl, msg=msg) for t, lvl, msg in [
    ("10:00:01", "INFO", "Starting minecraft server version 1.21.1"),
    ("10:00:06", "INFO", "Loading Minecraft 1.21.1 with Fabric Loader 0.17.2"),
    ("10:00:41", "INFO", 'Done (34.2s)! For help, type "help"'),
    ("10:12:03", "INFO", "Blankeos joined the game"),
    ("10:12:44", "INFO", "Talaba joined the game"),
    ("10:13:10", "INFO", "<Talaba> where are you?"),
    ("10:13:25", "INFO", "<Blankeos> at spawn, come over"),
    ("10:14:30", "INFO", "kuya_migs joined the game"),
    ("10:15:11", "INFO", "Talaba has made the advancement [Getting an Upgrade]"),
]]
PACKS = [
    {"project_id": "Jkb29YJU", "slug": "cobbleverse", "title": "COBBLEVERSE - Pokemon Adventure [Cobblemon]", "description": "Catch, battle and explore: a Cobblemon adventure pack with custom regions", "project_type": "modpack", "client_side": "required", "server_side": "required", "downloads": 1200000, "follows": 9000, "icon_url": None, "versions": ["1.21.1"], "categories": ["fabric"]},
    {"project_id": "Ax53wWwP", "slug": "cams-cobbleverse", "title": "Cam's CobbleVerse", "description": "A lighter Cobblemon pack for friends", "project_type": "modpack", "client_side": "required", "server_side": "required", "downloads": 310000, "follows": 900, "icon_url": None, "versions": ["1.21.1"], "categories": ["fabric"]},
]
MOD_HITS = [("Simple Voice Chat", "Proximity voice chat for your server, no client mods needed to host", 12400000),
            ("Spark", "A performance profiler for Minecraft clients, servers and proxies", 9100000),
            ("Chunky", "Pre-generates chunks, quickly and efficiently", 7300000),
            ("Lithium", "No-compromises game logic optimization mod", 21000000)]


class Daemon(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def send(self, code, body):
        body = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        n = int(self.headers.get("content-length") or 0)
        b = json.loads(self.rfile.read(n) or b"{}") if n else {}
        p = self.path.split("?")[0]
        if p == "/api/servers/a/console":
            return self.send(200, {"accepted": True, "message": ""})
        if p == "/api/servers/a/mods/install":
            return self.send(200, {"installed": [b.get("title")], "skipped": [], "restart_required": True})
        self.send(200, {"ok": True, "action": "x", "warnings": []})

    def do_GET(self):
        p, q = self.path.split("?")[0], self.path
        if p == "/api/system/storage":
            return self.send(200, {"total_bytes": 80 * 1024**3, "used_bytes": int(41.3 * 1024**3), "free_bytes": int(38.7 * 1024**3), "servers": [{"id": "a", "bytes": int(6.4 * 1024**3)}, {"id": "b", "bytes": int(14.2 * 1024**3)}, {"id": "c", "bytes": int(19.8 * 1024**3)}]})
        if p == "/api/servers":
            return self.send(200, SERVERS)
        if p == "/api/servers/a/logs":
            start = 0 if "after=" not in q else len(LOGS)
            return self.send(200, {"service": "hostable-a.service", "lines": 200, "text": "\n".join(LOGS[start:]), "cursor": "s=%d" % len(LOGS)})
        if p == "/api/servers/a/overview":
            return self.send(200, {"active": True, "game": "minecraft", "server_type": "fabric", "game_version": "1.21.1", "players": {"online": len(ONLINE), "max": 20, "names": ONLINE}, "players_note": None, "motd": "Cobbleverse SMP", "max_players": 20, "online_mode": True, "whitelist_enabled": True, "whitelist_count": len(WL), "ops_count": len(OPS), "mods_count": sum(1 for m in MODS if m[4]), "mods_kind": "mod"})
        if p == "/api/servers/a/players":
            return self.send(200, {"online": {"online": len(ONLINE), "max": 20, "names": ONLINE}, "whitelist_enabled": True, "online_mode": True, "whitelist": [{"name": n, "level": None, "clean": True} for n in WL], "ops": [{"name": n, "level": l, "clean": True} for n, l in OPS], "banned": []})
        if p == "/api/servers/a/mods":
            return self.send(200, {"kind": "mod", "server_type": "fabric", "game_version": "1.21.1", "loader": "fabric", "items": [{"filename": f if e else f + ".disabled", "enabled": e, "size": int(sz), "title": t, "version": v, "project_id": slug} for t, slug, v, f, e, sz in MODS]})
        if p == "/api/modrinth/search" and "modpack" in q:
            return self.send(200, {"hits": PACKS, "offset": 0, "limit": 20, "total_hits": len(PACKS)})
        if p == "/api/modrinth/search":
            hits = [{"project_id": "p%d" % i, "slug": t.lower().replace(" ", "-"), "title": t, "description": d, "project_type": "mod", "client_side": "optional", "server_side": "required", "downloads": dl, "follows": 1, "icon_url": None, "versions": ["1.21.1"], "categories": ["fabric"]} for i, (t, d, dl) in enumerate(MOD_HITS)]
            return self.send(200, {"hits": hits, "offset": 0, "limit": 20, "total_hits": len(hits)})
        if p.startswith("/api/modrinth/projects/") and p.endswith("/install"):
            pid = p.split("/")[4]
            pack = "modpack" in q
            return self.send(200, {"project_id": pid, "project_type": "modpack" if pack else "mod", "version_id": "4SKGla61", "version_number": "1.7.42" if pack else "2.5.24", "game_versions": ["1.21.1"], "loaders": ["fabric"], "file": {"project_id": pid, "version_id": "4SKGla61", "filename": "COBBLEVERSE 1.7.42.mrpack" if pack else "voicechat-fabric-2.5.24.jar", "url": "https://cdn.modrinth.com/x", "sha512": None, "size": 238975006 if pack else 1900000}, "dependencies": []})
        if p.startswith("/api/servers/") and p.endswith("/status"):
            return self.send(200, {"active": STATUS.get(p.split("/")[3], "inactive")})
        self.send(404, {"error": "not found"})


# ---------------------------------------------------------------- the walkthrough
# Each step: (chapter, keys, hold_ms). Keys are sent one by one; the screen is captured
# once after the last key. Named keys: enter, esc, tab, down, up. Anything else is typed.
TYPE = 110  # ms per typed character
SCRIPT = [
    # 1. server list
    (0, [], 2200),
    (0, ["down"], 600),
    (0, ["up"], 700),
    # 2. create a server: Modrinth modpack + one extra mod, then the dry run
    (1, ["n"], 1300),
    *[(1, ["tab"], 160) for _ in range(5)],
    (1, ["enter"], 600),
    *[(1, [c], TYPE) for c in "cobble"],
    (1, ["enter"], 1300),
    (1, ["enter"], 1300),
    (1, ["enter"], 600),
    *[(1, [c], TYPE) for c in "voice"],
    (1, ["enter"], 1200),
    (1, ["enter"], 1100),
    (1, ["esc"], 1200),
    (1, ["tab"], 500),
    (1, [" "], 1000),
    (1, ["enter"], 2200),
    # 3. manage an existing server: console, players, mods
    (2, ["esc"], 700),
    (2, ["enter"], 1300),
    (2, ["c"], 2400),
    (2, ["tab"], 2000),
    (2, ["tab"], 2600),
]

KEYS = {"enter": b"\r", "esc": b"\x1b", "tab": b"\t", "down": b"\x1b[B", "up": b"\x1b[A"}

# Theme colours (crates/hostable-tui/src/theme.rs, truecolor) -> palette slot names.
PALETTE = {"8d9199": "muted", "4a4d53": "faint", "e0663a": "accent", "4cc38a": "ok", "d9a441": "warn", "ec6a6a": "bad",
           "default": "", "white": "", "brightwhite": "", "black": "faint", "red": "bad", "green": "ok",
           "yellow": "warn", "brown": "warn", "brightblack": "muted"}


def slot(color):
    if color in PALETTE:
        return PALETTE[color]
    sys.stderr.write("unmapped colour %s, using default\n" % color)
    return ""


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def capture(binary):
    port = free_port()
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", port), Daemon)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    token = tempfile.NamedTemporaryFile("w", suffix=".token", delete=False)
    token.write("demo-token")
    token.close()

    pid, fd = pty.fork()
    if pid == 0:
        os.environ.update(TERM="xterm-256color", COLORTERM="truecolor")
        os.environ.pop("NO_COLOR", None)
        os.execv(binary, ["hostable", "--daemon-url", "http://127.0.0.1:%d" % port, "--operator-token", token.name])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)

    def pump(sec):
        end = time.time() + sec
        while time.time() < end:
            r, _, _ = select.select([fd], [], [], 0.03)
            if fd in r:
                try:
                    stream.feed(os.read(fd, 65536))
                except OSError:
                    return

    def snap():
        rows = []
        for y in range(ROWS):
            line = screen.buffer[y]
            cells = []
            for x in range(COLS):
                ch = line[x]
                flags = ("b" if ch.bold else "") + ("i" if ch.italics else "") + ("u" if ch.underscore else "") + ("r" if ch.reverse else "")
                cells.append((ch.data or " ", (slot(ch.fg), slot(ch.bg) if ch.bg != "default" else "", flags)))
            rows.append(cells)
        cursor = None if screen.cursor.hidden else [screen.cursor.x, screen.cursor.y]
        return rows, cursor

    frames = []
    pump(2.5)
    for chapter, keys, hold in SCRIPT:
        for k in keys:
            os.write(fd, KEYS.get(k, k.encode()))
            pump(0.08)
        # network-backed steps (search, open) need longer to settle than typing
        pump(0.25 if hold <= TYPE * 2 else 1.1)
        rows, cursor = snap()
        frames.append({"chapter": chapter, "hold": hold, "rows": rows, "cursor": cursor})
    os.kill(pid, 9)
    httpd.shutdown()
    os.unlink(token.name)
    return frames


def encode_row(cells, styles):
    # trim trailing default blanks, then run-length encode as [styleIndex, text, ...]
    while cells and cells[-1][0] == " " and cells[-1][1] == ("", "", ""):
        cells = cells[:-1]
    out, cur, buf = [], None, ""
    for ch, st in cells:
        plain = lambda s: s is None or (not s[1] and "r" not in s[2] and "u" not in s[2])
        if ch == " " and plain(st) and plain(cur):
            st = ("", "", "") if cur is None else cur  # a blank looks the same in any plain run
        if st != cur and buf:
            out += [styles.setdefault(cur, len(styles)), buf]
            buf = ""
        cur = st
        buf += ch
    if buf:
        out += [styles.setdefault(cur, len(styles)), buf]
    return out


def write_ts(frames, path):
    styles, rows, row_index = {("", "", ""): 0}, [], {}
    out_frames = []
    for f in frames:
        idx = []
        for cells in f["rows"]:
            enc = json.dumps(encode_row(cells, styles), ensure_ascii=False, separators=(",", ":"))
            if enc not in row_index:
                row_index[enc] = len(rows)
                rows.append(enc)
            idx.append(row_index[enc])
        out_frames.append([f["chapter"], f["hold"], f["cursor"], idx])
    style_list = [None] * len(styles)
    for st, i in styles.items():
        style_list[i] = list(st)
    with open(path, "w") as fh:
        fh.write("// Generated by scripts/capture-tui-demo.py from the real `hostable` TUI. Do not edit by hand.\n")
        fh.write("// styles: [fg slot, bg slot, flags b/i/u/r]; rows: [styleIndex, text, ...];\n")
        fh.write("// frames: [chapter, holdMs, cursor [x, y] | null, rowIndexes].\n")
        fh.write("export const COLS = %d\nexport const ROWS = %d\n" % (COLS, ROWS))
        fh.write("export const CHAPTERS = [\"Servers\", \"Create\", \"Manage\"]\n")
        fh.write("export const STYLES: [string, string, string][] = %s\n" % json.dumps(style_list, separators=(",", ":")))
        fh.write("export const LINES: (string | number)[][] = [\n%s,\n]\n" % ",\n".join(rows))
        fh.write("export const FRAMES: [number, number, [number, number] | null, number[]][] = [\n%s,\n]\n"
                 % ",\n".join(json.dumps(f, separators=(",", ":")) for f in out_frames))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=os.path.join(ROOT, "target/debug/hostable"))
    ap.add_argument("--out", default=os.path.join(ROOT, "docs/_components/tui-demo-frames.ts"))
    ap.add_argument("--text", action="store_true", help="print frames as plain text instead of writing --out")
    args = ap.parse_args()
    frames = capture(args.bin)
    if args.text:
        for i, f in enumerate(frames):
            print("===== frame %d  chapter %d  hold %d  cursor %s" % (i, f["chapter"], f["hold"], f["cursor"]))
            for cells in f["rows"]:
                print("".join(c for c, _ in cells).rstrip())
        return
    write_ts(frames, args.out)
    total = sum(f["hold"] for f in frames)
    print("%d frames, %.1fs loop -> %s (%d bytes)" % (len(frames), total / 1000, args.out, os.path.getsize(args.out)))


if __name__ == "__main__":
    main()

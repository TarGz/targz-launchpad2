# Mechanics sheet

The original is two programs in `Targz Launchpad.app/Contents/MacOS/`:

- `targz-launchpad` — the Rust/Tauri shell (window + menu-bar icon). Decompiled; addresses below.
- `server` — Node.js packed with pkg. Its JavaScript is stored as V8 bytecode, so the
  native decompiler can't read its logic; its constant pools (strings, regexes, lists) are
  readable and are cited by address. Small numbers (ports, timeouts) are bytecode immediates
  that aren't decodable here, so those come from the person's own source repo
  (`~/Documents/GIT/_targz-launchpad/server.js`), marked `server.js:<line>`.

## Menu-bar icon — `targz-launchpad`

| Rule | Value | Where |
|---|---|---|
| Menu: each running project | id `project_<port>`, label `<name> :<port>` (format_inner ×2) | `app_lib::tray::build_tray_menu` 0x10029dd44, loop at 0x10029dde8 (reads `running` flag at +0x18, port `Option<u16>` at +0x1a) |
| then | separator, `open_dashboard`/"Open Dashboard", separator, `quit`/"Quit" | 0x10029dd44 (strings 0x1006ff07c, "quit") |
| Separator after projects only when ≥1 running | yes | 0x10029dde0 |
| Poll period | 5 s (`mov w0,#5; mov w1,#0` → `tokio::time::sleep`) | setup_tray closure 0x1000c3c1c, call at 0x1000c45d4 |
| Poll source | `GET http://localhost:2800/api/projects` | 0x1000c3e08 → 0x1006e90ae |
| Change detection | running set → `"name:port"` sorted, joined with `;`; skip if unchanged | 0x1000c3c1c (strings `;`, sort calls `driftsort_main`) |
| Title / tooltip | running count / `Targz Launchpad - N running` or `- idle` | 0x1000c3c1c (string "Targz Launchpa…"), 0x1007ed28b |
| AppKit work on main thread | `AppHandle::run_on_main_thread` | 0x1000c3c1c |
| Project click | open `http://localhost:<port>` | setup_tray closure 0x10019ccd8 (string `project_` 0x1006efcb8) |
| Quit | `POST /api/shutdown` (stops every project) then exit | 0x1000c4a58 → 0x1006e908c |

## Startup — `targz-launchpad`

| Rule | Value | Where |
|---|---|---|
| Wait for server | port 2800 (`mov w9,#0xaf0`), max 30 s (`mov w8,#0x1e`) | `app_lib::run::{closure}` 0x1000c2c00, at 0x1000c2c58–0x1000c2c60 |
| HTTP timeout per try | 2 s (`mov w8,#2` into ClientBuilder) | 0x1000c2cf0 |
| Retry delay | 500 ms (`0x1dcd6500` ns) | 0x1000c3040 and 0x1000c318c |
| Then | navigate window to `http://localhost:2800`, set up tray | 0x1000c3420 → 0x1006e9077 |
| Resources | `Resources/_up_/index.html` | 0x1006eed8c |

The rebuild has no local HTTP server (the logic is in-process), so the 2800/30 s/2 s/500 ms
startup wait has nothing to wait for. Kept for reference, not used.

## Scan, start, stop — `server` (constant pools)

| Rule | Value | Where |
|---|---|---|
| Port rule | regex `--port[= ](\d+)` | 0x102d13c3f |
| Adopt running servers | `lsof -ti tcp:<port> -sTCP:LISTEN` | 0x102d140b3, 0x102d293dd |
| Config file | `~/.devlauncher.json`, key `autoPortStart` | 0x102d1da06 …, 0x102d16c61 |
| First auto port | 2850 | server.js:15 (bytecode immediate) |
| node start: strip | regex `\s*--open(?:=[^\s]*)?`, rewrite `--port=` | 0x102d16e41, 0x102d16e82 |
| node start: live-server | add ` --no-browser` if missing | 0x102d16e8d–0x102d16ec8 |
| node start: spawn | `sh -c`, `cwd`, `stdio: ignore`, PATH = `node_modules/.bin`+PATH, env `PORT_PROJECT`, `BROWSER`, `unref` | 0x102d16ed0–0x102d1700a |
| node health wait | 5000 ms | server.js:476 |
| jekyll start | `--port `, strip `\s*--livereload-port\s+\d+`, `\s*--livereload\b`, `\s*--open`, join lines with ` && ` | 0x102d1727c–0x102d17318 |
| jekyll fallback | `bundle exec jekyll serve --port ` if `Gemfile`, else `jekyll serve --port ` | 0x102d17392–0x102d173d6 |
| jekyll PATH | `.rbenv/shims`, `.rvm/bin`, `/opt/homebrew/opt/ruby/bin`, `/usr/local/opt/ruby/bin`; `/bin/sh` | 0x102d173f4–0x102d17462 |
| jekyll health wait | 15000 ms | server.js:558 |
| static start | `npx live-server . --port=<auto> --no-browser --quiet` | 0x102d17716 |
| static health wait | 5000 ms; poll every 300 ms | server.js:583, 604 |
| Editor order | Zed, Cursor, Visual Studio Code, VSCodium, Windsurf, Sublime Text, Nova, WebStorm, BBEdit | 0x102d1db2b–0x102d1dbc3 |
| Stop | SIGTERM to process group, then SIGKILL listeners on the port | server.js:612-627 |

Note: the installed `server` has no terminal launcher (`Terminass` is absent from its
constants), so the source repo is slightly newer than the installed app. The rebuild follows
the installed app: editor and Finder on each card, no terminal button.

## Dashboard — `Resources/_up_/index.html` (dark theme = Skew values)
- Window 1200×800 (tauri.conf.json). Content max 1200 px, padding 32/40; grid min column 340, gap 16.
- Card radius 16, small radius 10; lit card #1e2023, stopped #0e0f11; running rail #5EEAD4→#2DD4BF.
- Launch: teal ramp; Stop + Restart: one orange key; tools: metal; search: #0e0f11 well, teal text.

## Added for this rebuild (requested by the person)
- Self-update from git: check 8 s after start, then every 30 min (estimated, chosen here);
  `Update (N)` key → `git pull --ff-only`, `install.sh`, relaunch.
- Light mode: not built yet (request: dark only first).

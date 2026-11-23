<p align="center"><img src="assets/icon.png" width="120" alt="Explorateur logo"></p>

<h1 align="center">Explorateur</h1>

<p align="center">A fast, lightweight file explorer for Windows, written in Rust.<br>
Built for working alongside AI coding agents: files that change on disk show up <b>live</b>, without reloading the view.</p>

> The interface is in **French** (the code comments too). Everything below explains what it does.

## Download

1. Grab `Explorateur-vX.Y.Z-windows-x64.zip` from the [latest release](https://github.com/Desarrh/explorateur/releases/latest).
2. Unzip it anywhere and run `Explorateur.exe`. No installer, no admin rights, no runtime to install.
3. Windows SmartScreen may warn you the first time (the exe is not code-signed): *More info → Run anyway*.

The app checks GitHub for new releases once a day and shows an **Install** button in the status bar
when one is available: it downloads the new version, swaps the exe and restarts. You can turn this off in the `⋯` menu.

## Why

Windows Explorer gets painfully slow when a tool rewrites files in a folder you are looking at
(an AI agent, a build, a sync client): it re-enumerates the whole folder, reloads icons and shell
extensions on every burst of changes. Explorateur patches the list in place instead, and draws its UI
on the CPU so it stays light.

## Performance

All numbers measured on the author's machine (Windows 11, NVMe SSD, 5120×1440 display), release build.

| | |
|---|---|
| Executable | ~14 MB, single file |
| Memory, normal use | **~40–45 MB** (30 MB private in the headless 10,000-file benchmark) |
| Show a 10,000-file folder | ~40–60 ms to the first full frame, 2–4 ms per frame afterwards |
| Memory with the global search index (~2 M files) | ~160 MB right after the first scan (~80 MB of index) — can be turned off |
| Open `C:\Windows\System32` (5,944 entries) | ~8 ms |
| Global search over 1.97 M file names | **~5 ms per keystroke** |
| Full index build (all fixed drives, 1.97 M entries) | 27–37 s, in the background, saved to disk |

**Folder sizes** (warm disk cache on both sides, same totals):

| Folder | Naive scan (single thread, `read_dir` + `metadata`) | Explorateur |
|---|---|---|
| User profile (319 GB) | 25.6 s | **4.0 s** |
| `C:\Program Files` | 249 ms | **71 ms** |
| `C:\Windows\System32` | 127 ms | **36 ms** |

### How it stays fast and small

- **CPU rendering, no GPU context.** Like lightweight Linux file managers (PCManFM, Thunar), the UI is
  rasterized on the CPU ([egui](https://github.com/emilk/egui) + a tiled software renderer) and presented
  through GDI with [softbuffer](https://github.com/rust-windowing/softbuffer). Creating an OpenGL/DirectX
  context alone costs ~150 MB of driver memory; dropping it took the app from ~160 MB to ~40 MB.
- **Virtualized lists**: only the visible rows are laid out and drawn, whatever the folder size.
- **Live updates without reloading**: [notify](https://github.com/notify-rs/notify) (ReadDirectoryChangesW)
  events are coalesced per frame and patched into the list. Atomic writes (temp file + rename, what agents
  and editors do) don't flicker. Changed files are highlighted for a few seconds.
- **Folder sizes, WizTree-style without admin**: parallel work-stealing traversal (rayon), directories read
  in large batches with `FindFirstFileExW(FIND_FIRST_EX_LARGE_FETCH, FindExInfoBasic)` so sizes come with the
  enumeration, and every sub-folder size is cached so entering an already scanned folder is instant.
  (WizTree is even faster by reading the NTFS MFT directly, which requires admin.)
- **Global search, Everything-style without a service**: all file names of fixed drives in one compact
  lowercase arena (~40 bytes per file), searched with a single `memmem` pass; the real file-name case is
  read back only for the handful of results shown. (Everything stays live by reading the NTFS USN journal
  from an admin service; here the index is rebuilt at startup and every 6 hours, and the *Récents* tab
  covers the gap live. Never indexed: `Windows`, `WinSxS`, `.git`, `$Recycle.Bin`, `System Volume Information`
  and build folders such as `target`, `node_modules`, `obj`; removable and network drives are skipped.)
- **Thumbnails** are decoded on 2–4 worker threads: almost every image format (PNG, JPEG, WebP, AVIF, TIFF,
  SVG, PSD, HEIC/RAW through WIC…) with an on-disk QOI cache so the second visit is instant; videos / PDF /
  Office / album art through the Windows thumbnail provider (`IShellItemImageFactory`), like Explorer.
- **Only the Segoe UI fonts that are needed** are loaded: vector icons instead of the 13 MB emoji font.
- **Everything slow runs off the UI thread**: directory listings, file contents, copies/moves, git, zip,
  thumbnails, search, drive free space. The UI thread only does the occasional single-file check
  (does this path exist, is it a folder) after a user action or a change event.

## Features

**Browsing**
- Tabs, back/forward/up, clickable breadcrumb (or type a path), quick access, pinned folders, drives with free space
- Detailed list (natural sort, date, type, size) or thumbnail grid; preview pane (image, video frame, text/code)
- <kbd>Space</kbd>: quick look in Lecteur universel when it is installed (toggle in the `⋯` menu), otherwise the
  built-in full-screen preview, arrow keys to flip through files
- Dual pane (<kbd>F3</kbd>), <kbd>F6</kbd> / <kbd>Shift</kbd>+<kbd>F6</kbd> copy / move to the other pane
- Go to (<kbd>Ctrl</kbd>+<kbd>P</kbd>): fuzzy search over your frequent/recent folders **and every file on your drives**
- Fuzzy filter in the current folder (with history autocomplete), or recursive search that respects `.gitignore`
- Address bar autocomplete (sub-folders and frequent folders); command palette <kbd>Ctrl</kbd>+<kbd>K</kbd> with every action
- **Recent files everywhere**: a *Récents* tab listing the 300 files modified last on all your fixed drives
  (exports, captures, downloads), live. Skips AppData, ProgramData, Program Files, Steam libraries, temp, hidden
  and build folders, and `.log` / `.tmp` / `.part` files. Only there while "Recherche dans tous les disques" is on.
- **Disk-space map** (<kbd>Ctrl</kbd>+<kbd>3</kbd>), WizTree-style: the folder as nested rectangles sized by what they take,
  colored by type; click to dive in, right-click to open, show or send to the recycle bin
- Type-ahead, hidden files toggle, cream light theme / navy dark theme

**File operations**
- Copy / cut / paste compatible with Windows Explorer's clipboard, drag & drop in, **out to the desktop, Discord, browsers…**
- Rename (<kbd>F2</kbd>, extension preserved), **batch rename** (pattern `{nom} {n:3} {ext} {date}`, find/replace, regex, conflict preview)
- **Undo** (<kbd>Ctrl</kbd>+<kbd>Z</kbd>, last 100 steps) for copy, move, rename and batch rename, new folder or file,
  recycle bin, zip, unzip, conversions
- Background queue with progress and cancel; zip / unzip (safe paths only, size checked against free space)
- Files dropped from another app are always copied, never moved
- Full native Windows context menu (7-Zip, Git, "Send to"…) with <kbd>Shift</kbd>+right-click
- Open with, properties, terminal here, VS Code, **"Open Claude Code here"** (opens a new session in the Claude desktop app)

**Dev view** (switch with the Bureau / Dev toggle)
- Git changes with +/− line counts, checkboxes to pick what goes into the commit, diff of the selected file
- Commit (<kbd>Ctrl</kbd>+<kbd>Enter</kbd>), push, restore a file
- Live activity feed (last 300 changes): every file created / modified / deleted, files ignored by the root
  `.gitignore` hidden (nested `.gitignore` files are not read)
- **Agent passes**: changes made in a row (by an AI agent, a build…) are grouped into passes (20 s of silence
  closes one); see the combined diff of a pass, **undo the whole pass** (files back to the exact bytes they had
  before it) and redo it. Recorded only in a git repo, while the Dev view is open. Uses git snapshots through a
  temporary index: your index, branch and history are never touched, but `.git/objects` gets unreferenced objects
  (up to 5,000 files / 256 MB for the first snapshot; `git gc` removes them). Files over 64 MB are not restored.

**Windows integration**
- Can become the default file explorer (`⋯` menu): <kbd>Win</kbd>+<kbd>E</kbd> and folders opened from the desktop
  or other apps open here, as a new tab of the running window. Only per-user registry keys, fully reversible.
- Single window: launching it again opens a tab in the existing window (named pipe, < 1 ms, no polling)
- Pairs with [Lecteur universel](https://github.com/Desarrh/lecteur-universel): double-click media and documents to open them
  there, right-click → *Convertir en…* to convert. The list of formats comes from the viewer itself
  (`--formats`), with a built-in list as fallback for older versions
- Crash log in `%APPDATA%\Explorateur\crash.log`, reported on next launch
- Self-update is on trial: a new version that crashes, or starts twice without showing its window, is replaced by
  the previous one automatically, and is not offered again

Press <kbd>F1</kbd> in the app for all keyboard shortcuts.

## Build from source

Requires Rust 1.95+ (edition 2024) with the MSVC toolchain, on Windows 10/11.

```powershell
git clone https://github.com/Desarrh/explorateur
cd explorateur
cargo build --release        # target\release\explorateur.exe
cargo test --release         # unit tests
```

`scripts\maj.ps1` builds at low priority and replaces `dist\Explorateur.exe` (the running window keeps the old
version until it closes; `-Relancer` closes it and starts the new one, if it was running).
Benchmarks: `cargo test --release bench_affichage -- --ignored --nocapture`, `bench_taille` and `bench_index`.
Shared code (theme, icons, widgets, image decoders, settings, single instance, self-update) comes from
[socle](https://github.com/Desarrh/socle).

| File | Role |
|---|---|
| `src/main.rs` | winit window, event loop, CPU rendering, drag out, native menu |
| `src/app.rs` | State, tabs, selection, live watching, actions |
| `src/ui.rs` | Interface (virtualized list and grid, panes, menus) |
| `src/extras.rs` | Go-to palette, full-screen preview, batch rename |
| `src/commands.rs` | Command palette (<kbd>Ctrl</kbd>+<kbd>K</kbd>) and shortcut help |
| `src/sizes.rs` | Parallel folder sizes |
| `src/spacemap.rs` | Disk-space map (squarified treemap) |
| `src/index.rs` | Global file-name index (with modification dates) |
| `src/recents.rs` | Recent files on all drives |
| `src/bursts.rs` | Agent passes (grouping, snapshots) |
| `src/thumbs.rs` | Thumbnails (socle decoders + disk cache, Windows thumbnail provider) |
| `src/ops.rs` | File operation queue with undo |
| `src/git.rs` | Git status, diff, commit, push, pass snapshots |
| `src/winx.rs` | Windows clipboard, drag out, shell menu, default explorer |
| `src/fsx.rs` | Directory reading, file types, the one list of noisy folders |
| `src/viewer.rs` | Lecteur universel link (open, quick look, `--formats`, `--convert`) |
| `src/tests.rs` | Unit tests, benchmarks and headless screenshots |

## Known limitations

- Windows only.
- The global index is rebuilt every 6 hours: a file created since may be missing from <kbd>Ctrl</kbd>+<kbd>P</kbd>
  (the *Récents* tab, on the other hand, is live).
- Recursive search stops at 20,000 results; <kbd>Enter</kbd> opens at most 16 selected items; diffs are cut at
  20,000 lines.
- No built-in video playback (the preview shows a frame; click to play in your player or in Lecteur universel).
- The desktop and taskbar stay Windows Explorer's.

## License

MIT — see [LICENSE](LICENSE).

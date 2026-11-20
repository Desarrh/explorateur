<!--
EGM drift report. Documents checked: STACK.md, README.md (unchanged since 8c691af).
Code baseline: HEAD af66805 (version 0.3.1, socle tag v0.1.6), clean working tree, re-read on 2026-11-21.
Socle claims checked with `git -C C:\Dev\socle show v0.1.6:<path>`.
Line numbers are for af66805.
-->

## STACK.md

- **§3 Threads / §1 Principes: "boucle UI (principal, **aucun accès disque**)" and "tout ce qui touche au disque … tourne sur des threads".** In the code the UI thread calls the filesystem on every frame and on every action:
  - live events: `p.exists()` (src/app.rs:1116), `Entry::stat` / `fs::metadata` through `apply_path` (src/app.rs:1004), and `p.is_dir()` (src/app.rs:1139) inside `process_watch`;
  - single rename `fs::rename` plus `target.exists()` (src/app.rs:2150, 2154);
  - new folder or file `fs::create_dir` / `fs::write` and the `unique_name` probing loop (src/app.rs:2174, src/fsx.rs:212-227);
  - `Navigate` / `Open` / `FullView` `is_dir` / `exists` / `is_file` (src/app.rs:1979, 2028, 2391);
  - Ctrl+P: `Path::is_dir` on every candidate (src/extras.rs:54), and `exists()` plus `canonicalize` on up to 40 index hits per keystroke (src/extras.rs:76, 80);
  - batch-rename preview: `is_dir`, `metadata` and `exists` per file (src/extras.rs:328, 460, 470);
  - `rename_box`: `is_dir` (src/ui.rs:989);
  - full-screen view: `Entry::stat` (src/extras.rs:230);
  - the `winx::virtualized()` probe writes a file and scans `%LOCALAPPDATA%\Packages` (src/app.rs:2489 → src/winx.rs:143-155);
  - startup: `is_dir` on quick-access folders and saved tabs (src/app.rs:565, 681), and `viewer::find` `is_file` (src/viewer.rs:20).
  Fix: either move these calls to threads, or reword the doc to "no blocking enumeration; isolated `stat` calls are allowed".
- **§3 Flux de données: "draw() ne fait que **collecter des `Action`** … Le dessin ne modifie jamais l'état directement".** `draw` changes state and starts work directly:
  - theme and view mode: `set_dark` (src/ui.rs:201), `s.dev` (src/ui.rs:208);
  - the ⋯ menu starts or stops the global index and the live drive watcher (src/ui.rs:262-267), and writes the registry through `set_default_explorer` (src/ui.rs:290);
  - activity feed cleared (src/ui.rs:1448);
  - palette commands run immediately (src/ui.rs:1843 → `run_command`);
  - batch rename submitted to the queue from `draw_batch` (src/extras.rs:421-430);
  - "Convertir en" starts `lecteur --formats` threads while drawing (src/ui.rs:1075 → src/app.rs:1403);
  - the split pane swaps `self.active` (src/ui.rs:92-105).
  Fix: update the doc to describe the actual rule, or route these through `Action`.
- **Header (line 4): "commit `1cef86d`, version 0.2.0".** The code is at HEAD `af66805`, and `Cargo.toml:3` says `version = "0.3.1"`.
- **§2 Socle commun: "`socle` v0.1.0 (tag git …)", features list `ui images img-svg img-wic settings crash instance update`.** `Cargo.toml:30` uses `tag = "v0.1.6"` and also enables `testmode`. The doc also says "en local `.cargo/config.toml` le remplace par `../socle`": only `.cargo/config.toml.off` exists, so the git tag is what gets built.
- **§5 Mise à jour (socle::update): no mention of the trial and rollback added in socle v0.1.6.** The code calls `socle::update::rollback_on_panic()` and `startup()`, which brings the old exe back if the new one panics or starts twice without showing a frame (src/main.rs:271-272). The first frame confirms the version (`confirm_started`, src/main.rs:94). This is user-visible and undocumented.
- **§5 Réglages: "empreinte des réglages comparée au plus toutes les 3 s".** The fingerprint is now checked every 1 s (src/app.rs:720). The saver is `Saver::debounced(300 ms, 2 s)`: written 300 ms after the last change, at most 2 s after the first (src/app.rs:614).
- **§7 Lecteur universel: "« Convertir en » → `lecteur-universel --convert <fmt> <fichiers>`".** The menu entries now come from `lecteur --formats <ext>` (JSON, cached per extension). This only happens if the exe's version resource has `LecteurCli` containing `formats`; otherwise the built-in list is used (src/app.rs:1403-1447, src/viewer.rs:30-110). Not documented. The `unsafe` call to `GetFileVersionInfoW` also adds `viewer.rs` to the files that use `unsafe` (see the `unsafe` item).
- **§9 QoL #6: "tri partiel des 200 premiers seulement".** The tab shows `recents::LIMIT = 300` files (src/recents.rs:19), and the live watcher keeps up to 3 000 paths (`LIVE_MAX`, src/recents.rs:21).
- **§9 QoL #9: "disposition calculée sur un thread".** Only the tree read runs on a thread (src/spacemap.rs:275-278). The squarified `layout()` runs on the UI thread inside `draw_space_map` (src/spacemap.rs:320-322).
- **§9 header: "faites : 2, 6, 9".**
  - #4 (address and filter autocompletion) is done: `refresh_address_candidates` and `autocomplete_field` (src/app.rs ~760, src/ui.rs:355, 389). §6 lists it as working.
  - #5 (thumbnails for almost every format) is mostly done (src/thumbs.rs). EXR and JXL are not: the `img-exr` / `img-jxl` socle features are off in `Cargo.toml:30`.
  - Fix: mark 4 and 5 as done, and note what is missing from 5.
- **§5 Style de code: "`unsafe` limité aux appels Win32 de `winx.rs`, `sizes.rs`, `index.rs`, `thumbs.rs`".**
  - `sizes.rs` has no `unsafe`.
  - `unsafe` is also used in src/fsx.rs:129, 139, 307-333 (`GetLogicalDrives`, `GetDiskFreeSpaceExW`, `FindFirstFileExW`), src/app.rs:2547-2549 (clipboard clear), src/ops.rs:404 (free space) and src/viewer.rs:54 (version resource).
- **§4: "polices chargées par `app::install_fonts`".** There is no such function. The app calls `socle::theme::install_fonts` (src/app.rs:544).
- **§4 Couleurs: "Bouton de mode actif en sombre : `#2B4470` (en dur dans `ui.rs`)".** `ui.rs` has no such constant. The colour comes from socle `Palette::seg_on()` (socle v0.1.6 src/theme.rs:277) and is used by `socle::ui::segmented`.
- **§6 Aperçu rapide: "si le lecteur … sort en erreur dans la seconde".** The fallback window is 1 200 ms (src/viewer.rs:132).
- **§8: "`app.rs` (~2 000 lignes) et `ui.rs` (~1 700)".** They are now 2 600 and 1 913 lines.
- **§6 / §8 Rafales d'agent: limitations listed as "seulement dans un dépôt git, et pas pendant « Pause »". A third one is missing: passes are only recorded for the whole repo while the Dev view is on.** The repo gets its recursive watcher only when `s.dev` is true (src/app.rs:1043-1046). In Bureau view, only changes inside the folders open in tabs reach `bursts.record`.
- **§8: "les instantanés écrivent des objets git non référencés". It does not say that just browsing triggers them.**
  - `git rev-parse` runs on every change of folder, whatever the view (src/app.rs:1273).
  - The first time a repo is detected, the base snapshot starts (src/app.rs:1297), even in Bureau view. It hashes modified and untracked files (up to 5 000 files / 256 MB) into `.git/objects`.
- **§6 Ctrl+P: "index de tous les fichiers des disques". The index skips some folders.** Folders named `windows`, `winsxs`, `.git`, `$recycle.bin`, `system volume information`, `$windows.~bt`, `$winreagent` are never indexed, at any depth. So are build folders (`target`, `node_modules`, `obj`, `.cache`, `intermediate` …) (src/index.rs:333, src/fsx.rs:247-258). Only fixed drives are scanned (src/index.rs:323-325).
- **§6 Récents: ignore list given as "AppData, Program Files, dossiers cachés, builds et fichiers temporaires". The code skips more.** It also skips `ProgramData`, `steamapps`, `temp` / `tmp`, any folder starting with `$` or `.`, and files such as `*.log`, `*.lock`, `*.part`, `*.crdownload`, `*.etl`, `~$*`, `desktop.ini` (src/fsx.rs:262-267, src/recents.rs:37-44). The open Récents tab reloads at most every 2 s (src/app.rs:891).
- **§2 Images: the format list "png, jpeg, gif, webp, bmp, ico, tiff, qoi, svg, + WIC" is incomplete.** socle `images` also decodes TGA, DDS (BC1-3), HDR, PSD/PSB and RAW embedded previews (socle Cargo.toml `images` feature, src/images/mod.rs:54-59).
- **§4 Icônes: the toolbar and sidebar lists are incomplete.** The toolbar also has `Chart` (space map, src/ui.rs:215), `SearchDeep` (recursive search, src/ui.rs:347) and `Plus` (new tab, src/ui.rs:163). Tabs and the sidebar use `Clock` for Récents (src/ui.rs:130, 547).
- **§4 Typographie: "nom de fichier 13,5".** Grid cells use 13.0 for the name and 11.5 for the meta line (src/ui.rs:966, 970).
- **§5 Raccourcis: shortcuts in the code but not in the doc.**
  - Ctrl+Maj+Tab: previous tab (src/app.rs:1810).
  - Alt+↑: parent folder (src/app.rs:1861).
  - Mouse back / forward buttons (src/app.rs:1855-1858).
  - Home / End / PgUp / PgDn, with Maj to extend the selection (src/app.rs:1916-1923).
  - Échap: clears the filter first, then the selection.
  - Entrée in the filter opens the first result (src/ui.rs:367).
  - In Ctrl+P, Entrée on a file shows it in its folder; Ctrl+Entrée opens a folder in a new tab or opens the file (src/extras.rs:214-219).
  - Ctrl+Entrée commits in the Dev view (src/ui.rs:1524).
- **Undocumented limits (user-visible, in neither doc):**
  - recursive search stops at 20 000 results (src/app.rs:1579);
  - undo stack keeps 100 entries (src/app.rs:1248);
  - Entrée opens at most 16 selected items (src/app.rs:1898);
  - activity feed keeps 300 entries and merges repeats of the same file within 4 s (src/app.rs:1172, 1190);
  - at most 40 passes are kept (src/bursts.rs:13), and an expanded pass lists 500 files (src/ui.rs:1719);
  - diffs are cut at 20 000 lines (src/git.rs:744);
  - "+N" counts for untracked files cover only the first 200 files under 2 MB (src/git.rs:686-694);
  - visit history is capped at 2 000 folders, with the 500 oldest dropped (src/app.rs:921-927);
  - 700 thumbnail textures (src/thumbs.rs:20);
  - the folder-size cache is cleared in one go above 300 000 entries (src/sizes.rs:18, 89-91);
  - space map: items under 0.2 % are grouped, at most 150 children per folder (src/spacemap.rs:18, 66).
- **Undocumented behaviours:**
  - files dropped from another app are always copied, never moved, and drops from the current folder are ignored (src/app.rs:1690-1697);
  - pasting after a cut empties the clipboard (src/app.rs:2109);
  - the space map is not shown while a filter is typed, or in the Récents tab (src/ui.rs:710);
  - the Récents entry and its live watcher exist only when the global index is on (src/ui.rs:546, src/app.rs:693-696).
  - §8 says "sans la recherche globale, pas de récents", but the ⋯ menu label "Recherche dans tous les disques" does not tell the user that unticking it also removes Récents.
- **§2 Langage: "Rust 1.95+".** `Cargo.toml` has no `rust-version`, so the minimum is neither declared nor enforced.

## README.md

- **"How it stays fast" (line 65): "the index is rebuilt at startup and every 20 minutes". Known limitations (line 151): "The global index is rebuilt every 20 minutes".** The code rebuilds every 6 hours (`RESCAN = 6 * 3600 s`, src/index.rs:17). It rewrites `index.bin` only when the names changed or once a day (src/index.rs:20, 295). Fix the README (STACK.md already says 6 h).
- **Features (line 77): "Full-screen preview with Space".** With the default settings, Space opens a quick look in Lecteur universel (`--apercu`) when the viewer is installed. The built-in full-screen view is only the fallback (`quick_viewer: true` default, src/app.rs `Settings::default`; src/app.rs:2391; src/viewer.rs:131-166). The README does not mention this, nor the ⋯ toggle "Espace : aperçu rapide dans le lecteur" (src/ui.rs:254).
- **Features (lines 82-83), Recent files everywhere: "live, skipping AppData, Program Files, hidden and build folders".**
  - The tab exists only while global search is on (src/ui.rs:546, src/app.rs:693-696). Line 35 says the index "can be turned off" without saying that this also removes Récents.
  - It shows at most 300 files (src/recents.rs:19).
  - It also skips ProgramData, steamapps, temp folders and `.log` / `.tmp` / `.part` files (src/fsx.rs:262-267, src/recents.rs:37-44).
- **Features (line 79): "every file on your drives". "How it stays fast" (line 62): "all file names of fixed drives".** Some folders are never indexed, at any depth: `Windows`, `WinSxS`, `.git`, `$Recycle.Bin`, `System Volume Information`, and build folders such as `target`, `node_modules`, `obj` (src/index.rs:333, src/fsx.rs:247-258). Removable and network drives are not indexed either (src/index.rs:323-325).
- **File table (lines 131-146) is missing three files.** It lists 14 of the 17 source files: `src/fsx.rs`, `src/viewer.rs` (Lecteur universel link, `--formats` / `--convert`) and `src/tests.rs` are missing.
- **Dev view (line 99): "Live activity feed: every file … git-ignored files hidden".**
  - Only the root `.gitignore` and `.git/info/exclude` are applied (src/app.rs `Repo::new`, ~255-260). Files ignored by a nested `.gitignore` still show up.
  - The feed keeps 300 entries and merges repeats within 4 s (src/app.rs:1172, 1190).
- **Dev view (lines 100-102): Agent passes are described without their rules and limits.**
  - A pass closes after 20 s of silence (src/bursts.rs:11).
  - Passes are only recorded in a git repo, while the Dev view is on (src/app.rs:1043-1046), and not while paused.
  - Files over 64 MB are not restored (src/git.rs:280).
  - The base snapshot hashes up to 5 000 untracked files / 256 MB (src/git.rs:282-283) into `.git/objects` as unreferenced objects. It starts as soon as a repo folder is opened, even in the Bureau view (src/app.rs:1297).
  - "Your index, branch and history are never touched" is true, but `.git/objects` grows. The README should say so.
- **File operations (line 91): "Undo … for copy, move, rename, new folder, recycle bin, zip". The list is incomplete.** New text file (src/app.rs:2166-2177), unzip (src/ops.rs:262-266) and "Convertir en" results (src/ops.rs:195-202) can also be undone.
- **Download (lines 16-17), the self-update description, does not cover the trial and rollback.** Since socle v0.1.6, a new version that panics, or starts twice without drawing a frame, is replaced by the previous exe and relaunched. That version is then no longer offered (src/main.rs:94, 271-272).
- **Windows integration (line 109): "right-click → Convertir en… to convert (PNG, WEBP, PDF, MP3, GIF…)".** The menu is now built from the viewer's `--formats` answer when the viewer supports it, with labels and tooltips from the viewer. The built-in list is the fallback (src/app.rs:1403-1447, src/ui.rs:1075).
- **Build from source (line 116): "Requires Rust 1.95+".** `Cargo.toml` has no `rust-version`, so the requirement is not declared.
- **Build from source (lines 125-126): "`-Relancer` restarts it".** `scripts\maj.ps1` relaunches only if a copy of `dist\Explorateur.exe` was running (scripts/maj.ps1:24-25, 48). Otherwise nothing is started.
- **User-visible limits that are not documented:**
  - recursive search stops at 20 000 results (src/app.rs:1579);
  - undo keeps 100 steps (src/app.rs:1248);
  - Entrée opens at most 16 selected items (src/app.rs:1898);
  - diffs are cut at 20 000 lines (src/git.rs:744);
  - files dragged in from other apps are always copied, never moved (src/app.rs:1697).

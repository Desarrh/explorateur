# Stage 6: Comprehension Test

**Target:** `C:\Dev\explorateur\STACK.md`. FeatureRange 1-361, DetailedRange N/A.
**Reviewer:** `egm-comprehension` (completed). Read the target document only, no GoldfishContext, as the protocol requires.

## What this proposes

STACK.md is a reference document for "Explorateur", a Windows file explorer written in Rust. It is built for working alongside AI agents that change files all the time, so changes on disk show up live in the view. It describes the current state of the code at commit `1cef86d` (v0.2.0). That covers the dependency stack, the module layout, the data and thread model, the visual design tokens, coding and runtime conventions (settings, updates, default-explorer registration, test mode), the features that work today, integrations with sibling apps, and known limits. It ends with a ranked backlog of 15 quality-of-life ideas, each tagged "novel" or "exists elsewhere" and given an effort size and an explanation of how it stays instant. Items 2, 6 and 9 are marked done.

## How the existing system works, per this document

One UI thread runs a hand-written winit loop. Each frame, `App::tick()` drains channels fed by background threads (folder loads, `notify` events, git, operations, thumbnails, sizes, index, updates, folders handed over by a second instance). Then `handle_keys()` runs. Then `draw()` collects `Action` values, which `apply_actions()` applies afterwards, so drawing never changes state. The UI thread never touches the disk. Rendering is CPU-only: egui output goes through a third-party software rasterizer, and softbuffer presents it with GDI. Shared pieces (theme, icons, widgets, settings, crash log, single instance, updater) come from an external `socle` crate. Git features shell out to `git`. "Agent bursts" group file changes into "passes" (closed after 20 s of silence) and take snapshots through a temporary git index, so a pass can be undone and redone. A global filename index and a recursive watcher on all fixed drives feed Ctrl+P and the "Récents" tab.

**Elephant note:** the paraphrase matches the document's intent. The reader got the architecture right. The gaps are undefined terms and internal contradictions, listed below.

## Findings

| ID | Sev | Lines / section | What a fresh reader cannot understand | Proposed change |
|---|---|---|---|---|
| CO-6-1 | Major | L32, 44, 109, 124, 144, 148, 156, 164, 230 | The Bureau / Dev mode is never defined. "Bureau" also means the Windows Desktop at L11 and L221. | In §1 or §6: "Two modes, switched by Ctrl+Maj+D or the segmented control: **Bureau** = normal browsing; **Dev** = adds the git panel (changes, commit, diff, push) and the live activity feed." Write "mode Bureau" where it could be confused with the Desktop. |
| CO-6-2 | Major | L51, 53, 54, 218, 231, 242, 245, L296 | Codes E1/E2/E3/T3 are undefined and clash with §9's 1-15 numbering. | Define once: "Ex/Tx = IDs in `C:\Dev\QOL.md` (E1 = §9 item 2, E2 = item 6, E3 = item 9, T3 = quick preview)", or drop the codes. |
| CO-6-3 | Major | L57 vs L220 | L57 says `viewer.rs` previews "par tube" (by pipe); L220 says the app never talks to the viewer's pipe. Elephant check: `viewer.rs` L124-145 spawns `lecteur-universel --apercu` and never opens the pipe, so L57 is stale. | L57: "quick preview by launching `lecteur-universel --apercu <file>`, which forwards to its open instance through its own pipe". |
| CO-6-4 | Major | L53, 69, 215, 242, 271-273, 287 | "Recherche globale" is undefined: which setting, default, and that it is the same as the "désactivable" index. Elephant check: the setting is `Settings::global_index` (`app.rs` L56). | One sentence in §5 or §6: "Global search (setting `global_index`, default <on/off>) = Ctrl+P filename index + watcher on all fixed drives. Récents depends on it. Turning it off saves ~160 MB RAM." |
| CO-6-5 | Major | L13 vs L272 | "~40-45 MB" principle vs "~160 MB after first scan". | L13: "~40–45 MB without the global index; ~160 MB just after the first index scan (see §8)". |
| CO-6-6 | Major | L296, L316-326 vs L27, 209-213 | §9 items 4 and 5 are listed as not done but §6 describes them as working. | Mark 4 and 5 ✅ (or partly done with what is left, e.g. TGA/DDS/EXR/JXL); heading "faites : 2, 4, 5, 6, 9". |
| CO-6-7 | Major | L15 vs L357 | Item 14's admin service contradicts "pas de droits admin" without saying so. | Add to item 14: "Breaks the no-admin / no-installer principle (§1); would need an optional, separately installed service; to be decided." |
| CO-6-8 | Major | L260-265, §7 | Launcher, YouTube client and Vigie are not explained or linked. | One-line definition and path/link for each. |
| CO-6-9 | Major | L231-241, 284-286 | When snapshots are taken is never stated, so L286 ("pass opened before the first snapshot has no diff") cannot be understood. | One sentence: "A base snapshot is taken when <trigger>; each pass gets a before snapshot at its first event and an after snapshot when it closes." |
| CO-6-10 | Major | L284 | "Pause" is undefined. | "…not while the activity feed is paused (« Pause » button in the Dev view, which stops recording events)" or the real behaviour. |
| CO-6-11 | Major | L196 | "Règle n° 0" is cited but not stated or linked. | State inline ("tests never disturb the user: no visible window, no focus, no real keyboard/mouse, never the user's instance or drives") and link `C:\Dev\DIRECTION.md` §0. |
| CO-6-12 | Nit | L36, 175-180, 191 | Relationship between `scripts\maj.ps1`, `dist\` and the release zip is unclear. | "`maj.ps1` = developer script: builds release, copies to `dist\`, restarts (`-Relancer` via `explorer.exe`). End users use the release zip and the built-in updater." |
| CO-6-13 | Nit | L189-194 | Why the Claude app matters (MSIX container) is not said. | Open with "Development often runs from the Claude desktop app, packaged as MSIX; anything it launches inherits its container:". |
| CO-6-14 | Nit | L85 | Design canvas reference not reachable. | Add a link or "(design file, not public; option B chosen)". |
| CO-6-15 | Nit | L47, 52, 82, 180, 185 | Small undefined terms: `EXPLIDX2`, `--apres-maj`, `AncienDefaut`/`AncienWinE`, "dossiers bruyants", `taille`/`arrondi`/`dim`. | One clause each (format tag; what the flag does; registry key; which folders; module vs function). |
| CO-6-16 | Nit | L1-4, 299-301 | Audience not stated; §9 addresses "tu" without introducing the user. | L3: "Audience: contributors and coding agents. §9 is ranked for the main user's uses (dev with AI agents, gaming, YouTube)." |

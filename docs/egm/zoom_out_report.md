# Stage 5: Initial Zoom Out Review

**Target:** `C:\Dev\explorateur\STACK.md` (full file, lines 1-361)
**GoldfishContext:** `C:\Dev\explorateur\README.md`, `C:\Dev\DIRECTION.md`. Linked files: `README.md` (already included). Left out (headless run, URLs not approved): `https://github.com/Desarrh/socle`, `https://github.com/Desarrh/lecteur-universel`.
**Reviewers:** `egm-zoom-out` (completed), `egm-inventive-catalyst` (completed).
**Deviation from protocol:** because of the run's reviewer budget (6 subagents max), both personas ran in one fresh subagent, as two separate audits. The standard asks for one subagent per persona.

**Elephant spot checks (done in the main context against the code):** `Cargo.toml` version is `0.3.0` (doc says 0.2.0); `app.rs` is 2,542 lines and `ui.rs` 1,909; no `2B4470` in `src/`; `unsafe` appears in `fsx.rs` (6), `app.rs` (3), `ops.rs` (1), `index.rs` (1), `thumbs.rs` (2), `winx.rs` (21) and `sizes.rs` (0); `main.rs` calls `rollback_on_panic`, `startup()`/`RolledBack` and `confirm_started`; `tests.rs` has 6 `#[ignore]` tests. All confirm the reviewer's facts.

## Part A: Zoom Out (Skeptical Architect)

| ID | Sev | Lines / section | Finding | Proposed change |
|---|---|---|---|---|
| ZO-5-1 | Major | L296-361, §9 | §9 is a second idea backlog next to `C:\Dev\QOL.md` (DIRECTION.md §7 says new features go to QOL.md). It numbers ideas 1-15 while §6 uses QOL IDs (E1, E2, E3, T3): two ID schemes in one doc. | Move open §9 items to `QOL.md` under its IDs; replace §9 with a pointer listing the explorer IDs. Keep the "⚡" notes as a QOL.md column if wanted. |
| ZO-5-2 | Major | L296, L308-341, §9 | Header says only 2, 6, 9 are done, but 4 (autocomplete, §6 L212-213) and 5 (image thumbnails, §6 L209) are shipped. Done items keep stale design text (item 6 "200 premiers" vs caller-supplied `limit` in `recents::collect`). | Mark 4 and 5 done; reduce shipped items to one-line pointers to §6 (moot if ZO-5-1 applied). |
| ZO-5-3 | Major | L357-359, §9 item 14 | "Petit service admin" contradicts §1 L15 "pas de droits admin" and DIRECTION.md rules 2 and 5 (no admin, no imposed background service). | Drop item 14, or restate as an opt-in elevated helper that is never required, with the rule conflict flagged for the user. |
| ZO-5-4 | Major | L303-307, §9 item 1 | "On each notify event, copy the old version": `notify` fires after the write, so the old content is gone. Also rebuilds the hash store E1 already has (git objects, `hash-object`, `cat-file --batch`). | Rewrite as a timeline over E1 pass snapshots; for non-git folders, use a baseline snapshot taken when watching starts (e.g. private bare repo). Remove the "copy on event" wording. |
| ZO-5-5 | Major | L87-109, §4 | Colour table duplicates `socle::theme` / `socle/STACK.md` §4 (DIRECTION.md §3 says that is the single source). Already drifted: L109 says `#2B4470` is hardcoded in `ui.rs`; it is in `socle/src/theme.rs`. | Replace L87-109 with a link to `socle/STACK.md` §4; keep explorer-only items (dimensions, logo colours, `MARK_SECS`). Fix or delete the `#2B4470` sentence. |
| ZO-5-6 | Major | L4, L279 | Doc claims "everything below is taken from main" at 0.2.0 / `1cef86d`; `Cargo.toml` is 0.3.0; module sizes are 2,542 / 1,909, not ~2,000 / ~1,700. | Update the version and commit (DIRECTION.md §7 asks for this each wave) or drop the hash; use relative sizes or refresh the counts. |
| ZO-5-7 | Major | L172, §5 Style de code | `unsafe` list is wrong: `sizes.rs` has none; `fsx.rs`, `app.rs`, `ops.rs` have some. | State the rule without a file list ("unsafe limited to Win32/COM calls"), or fix the list; optionally move the `app.rs` clipboard and `ops.rs` disk-space calls into `winx.rs`/`fsx.rs`. |
| ZO-5-8 | Major | L178-180, §5 Lancement / mise à jour | Update description omits the trial run with automatic rollback (`rollback_on_panic`, `startup()`→`RolledBack`, `confirm_started` in `main.rs`), the key safety property required by DIRECTION.md §4. | Add: "the new version runs on trial; if it panics before the first frame, `socle::update` rolls back to `.old` automatically." |
| ZO-5-9 | Major | L353-356, §9 item 13 | "Watcher already in place" is false: the watcher covers open tabs + current repo only (L67-68). Live status for every sidebar repo needs more watchers. | State the real cost (one watcher per pinned repo, capped, debounced) or reuse the fixed-drive recursive watcher from Récents filtered by repo root. |
| ZO-5-10 | Nit | L66-73, §3 Threads | Thread list (~20 kinds in one sentence) buries the key fact and will go stale. | Keep long-lived workers; one line for short-lived per-task threads. |
| ZO-5-11 | Nit | L141-149, §4 Composants | Segmented control listed under socle and again under explorer components. | Delete the explorer-side mention (or flag the duplicate implementation). |
| ZO-5-12 | Nit | L189-194, §5 Conteneur MSIX | Describes the developer's Claude/MSIX environment, already in DIRECTION.md §0. | Keep only product behaviour (`winx::virtualized()` refuses registry writes); point to DIRECTION.md §0. |
| ZO-5-13 | Nit | L332, §9 item 7 | Personal folder name `Vidéos\PsychoNight_Shorts` in a public repo. | Use `Vidéos\Shorts`. |
| ZO-5-14 | Nit | L345-348, §9 item 11 | Media Foundation decoding/audio in the explorer duplicates the lecteur universel and adds weight. | Reuse the lecteur's standby instance; show nothing if absent. |
| ZO-5-15 | Nit | L312-315, §9 item 3 | Relies on an unverified `q` URL parameter in the Claude app. | Mark as unverified; fallback: open the folder without a draft. |
| ZO-5-16 | Nit | L58, §3 | `tests.rs` has 6 ignored tests (3 benches + 3 `capture_*`), not 3. | "3 benchmarks + 3 headless captures, all `--ignored`". |

## Part B: Inventive Catalyst (TRIZ)

| ID | Sev | Lines / section | False compromise | Proposed resolution | Secondary costs |
|---|---|---|---|---|---|
| IC-5-1 | Major | L271-273 §8, L357-359 §9 | 6 h full rescan with two indexes in memory (~160 MB) vs an admin USN service. | Feed the existing fixed-drive `notify` watcher (Récents) into the index as in-place patches (delta + tombstones); rescan becomes a daily safety net or runs on buffer overflow; rebuild becomes a merge. | Delta bookkeeping in `index.rs`; buffer overflow must trigger a rescan; only with global search on. |
| IC-5-2 | Major | L231-241 §6, L284 §8, §9 items 1 and 8 | Each history feature brings its own store; passes need a git repo. | One content-addressed snapshot service extracted from `bursts.rs`/`git.rs`; private bare repo under `%LOCALAPPDATA%\Explorateur\` for non-git folders. Passes, rewind and image before/after read from it. | Disk use and GC policy; baseline scan cost; one subsystem shared by three features. |
| IC-5-3 | Nit | L290-291 §8 | 400-folder cap and slower than WizTree. | Seed from `sizes.rs` cache, draw coarse layout, refine visible area progressively. | Reflow as sizes arrive; more layout code. |
| IC-5-4 | Nit | L293-294 §8 | Stale parent sizes to avoid disk access for ignored events. | Mark parent size "dirty" in memory, show "≈", recompute lazily. | One state bit; a visual marker to explain. |

No false compromise found in the core architecture (CPU rendering, deferred actions, UI thread without disk access).

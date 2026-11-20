//! Fenêtres annexes : palette « Aller à » (Ctrl+P), aperçu plein écran (Espace),
//! renommage en lot.
use crate::app::{Action, App};
use crate::fsx;
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Frame, Margin, Rect, RichText, Sense, Stroke, Ui};
use socle::icons::{self, Icon};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

impl App {
    // ---------- Aller à (Ctrl+P) ----------
    pub fn draw_goto(&mut self, ui: &mut Ui) {
        if self.goto.is_none() {
            return;
        }
        let p = self.pal;
        let ctx = ui.ctx().clone();
        // Candidats : dossiers visités, épinglés, accès rapide, disques, sous-dossiers du dossier courant.
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let mut cands: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
        for (k, (n, last)) in &self.s.visits {
            let age_days = ((now - last).max(0) as f32) / 86_400.0;
            let frec = (1.0 + *n as f32).ln() * 12.0 + 30.0 / (1.0 + age_days);
            cands.insert(k.clone(), frec);
        }
        for path in self.s.pinned.iter().chain(self.quick.iter().map(|q| &q.1)) {
            cands.entry(path.to_string_lossy().into_owned()).or_insert(8.0);
        }
        for d in &self.drives {
            cands.entry(d.root.to_string_lossy().into_owned()).or_insert(2.0);
        }
        for e in self.tab().entries.iter().filter(|e| e.is_dir) {
            cands.entry(e.path.to_string_lossy().into_owned()).or_insert(4.0);
        }
        let g = self.goto.as_mut().unwrap();
        let q = g.query.trim().to_lowercase();
        let mut scored: Vec<(f32, String)> = cands
            .into_iter()
            .filter_map(|(path, frec)| {
                if q.is_empty() {
                    return Some((frec, path));
                }
                let low = path.to_lowercase();
                let base = Path::new(&path).file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
                // Le nom du dossier compte plus que le chemin complet
                let s_base = socle::fuzzy::score(&base, &q).map(|s| s as f32 + 400.0);
                let s_full = socle::fuzzy::score(&low, &q).map(|s| s as f32);
                s_base.or(s_full).map(|s| (s + frec * 3.0, path))
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let folders: Vec<String> = scored.into_iter().map(|x| x.1).filter(|p| Path::new(p).is_dir()).take(if q.len() >= 2 { 6 } else { 12 }).collect();
        // Index global : tous les fichiers des disques (recalculé seulement quand la saisie change)
        let (indexed, scanning) = match &self.index {
            Some(ix) => (ix.data.read().map(|d| d.len()).unwrap_or(0), ix.scanning.load(std::sync::atomic::Ordering::Relaxed)),
            None => (0, false),
        };
        if g.hits_q != q {
            g.hits_q = q.clone();
            g.hits = match &self.index {
                Some(ix) if q.len() >= 2 => ix
                    .data
                    .read()
                    .map(|d| d.search(&q, 40))
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|h| !folders.iter().any(|f| f.to_lowercase() == h.path.to_string_lossy()))
                    .filter(|h| h.path.exists())
                    .take(12)
                    .map(|h| {
                        // vraie casse, relue seulement pour les résultats affichés
                        let path = crate::index::real_case(&h.path);
                        let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or(h.name);
                        (path, h.is_dir, name)
                    })
                    .collect(),
                _ => vec![],
            };
        }
        // Liste unique : (chemin, dossier ?, nom affiché, vient de l'index ?)
        let mut results: Vec<(PathBuf, bool, String, bool)> = folders
            .iter()
            .map(|f| {
                let pb = PathBuf::from(f);
                let name = pb.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| f.clone());
                (pb, true, name, false)
            })
            .collect();
        results.extend(g.hits.iter().map(|(p, d, n)| (p.clone(), *d, n.clone(), true)));
        if g.sel >= results.len() {
            g.sel = results.len().saturating_sub(1);
        }

        // (chemin, ctrl, dossier ?)
        let mut go: Option<(PathBuf, bool, bool)> = None;
        let mut close = false;
        let (up, down, enter, esc, ctrl) = ctx.input_mut(|i| {
            use egui::{Key, Modifiers as M};
            (
                i.consume_key(M::NONE, Key::ArrowUp),
                i.consume_key(M::NONE, Key::ArrowDown),
                i.key_pressed(Key::Enter),
                i.consume_key(M::NONE, Key::Escape),
                i.modifiers.command,
            )
        });
        if up {
            g.sel = g.sel.saturating_sub(1);
        }
        if down && g.sel + 1 < results.len() {
            g.sel += 1;
        }
        if esc {
            close = true;
        }
        if enter {
            if let Some(r) = results.get(g.sel) {
                go = Some((r.0.clone(), ctrl, r.1));
            } else if Path::new(g.query.trim()).is_dir() {
                go = Some((PathBuf::from(g.query.trim()), ctrl, true));
            }
        }

        let screen = ctx.content_rect();
        let w = 640.0_f32.min(screen.width() - 32.0);
        // fond assombri, clic = fermer
        let backdrop = egui::Area::new(egui::Id::new("goto_backdrop")).order(egui::Order::Foreground).fixed_pos(screen.min).show(&ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, CornerRadius::ZERO, Color32::from_black_alpha(90));
            resp.clicked()
        });
        if backdrop.inner {
            close = true;
        }
        egui::Area::new(egui::Id::new("goto"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos2(screen.center().x - w / 2.0, screen.top() + 90.0))
            .show(&ctx, |ui| {
                Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(10))
                    .shadow(egui::Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha(70) })
                    .show(ui, |ui| {
                        ui.set_width(w);
                        ui.horizontal(|ui| {
                            let (ir, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                            icons::paint(ui.painter(), ir, Icon::Folder, p.accent);
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut g.query)
                                    .hint_text("Aller à un dossier… (tape quelques lettres)")
                                    .frame(Frame::NONE)
                                    .font(FontId::proportional(17.0))
                                    .desired_width(f32::INFINITY),
                            );
                            if g.fresh || !r.has_focus() {
                                r.request_focus();
                                g.fresh = false;
                            }
                            if r.changed() {
                                g.sel = 0;
                            }
                        });
                        ui.add_space(6.0);
                        if results.is_empty() {
                            ui.label(RichText::new("Rien ne correspond. Entrée ouvre un chemin tapé en entier.").color(p.muted));
                        }
                        let mut shown_header = false;
                        for (k, (pb, is_dir, name, from_index)) in results.iter().enumerate() {
                            if *from_index && !shown_header {
                                shown_header = true;
                                ui.add_space(4.0);
                                ui.label(RichText::new("SUR LES DISQUES").size(11.0).strong().color(p.muted));
                            }
                            let (rect, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
                            let on = k == g.sel;
                            if on || resp.hovered() {
                                ui.painter().rect_filled(rect, CornerRadius::same(8), if on { p.sel_bg } else { p.hover_bg });
                            }
                            let icon = if *is_dir { Icon::Folder } else { Icon::for_extension(pb.extension().and_then(|e| e.to_str()).unwrap_or("")) };
                            let ir = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(17.0, 17.0));
                            icons::paint(ui.painter(), ir, icon, if *is_dir { p.accent } else { p.muted });
                            ui.painter().text(pos2(rect.left() + 38.0, rect.top() + 12.0), Align2::LEFT_CENTER, name, FontId::proportional(14.5), p.text);
                            let parent = pb.parent().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.painter().text(pos2(rect.left() + 38.0, rect.top() + 29.0), Align2::LEFT_CENTER, &parent, FontId::proportional(11.5), p.muted);
                            if resp.clicked() {
                                go = Some((pb.clone(), ui.input(|i| i.modifiers.command), *is_dir));
                            }
                        }
                        ui.add_space(4.0);
                        let idx_info = match (&self.index, indexed, scanning) {
                            (None, _, _) => "index global désactivé (menu ⋯)".to_string(),
                            (Some(_), 0, true) => "index des disques en construction…".to_string(),
                            (Some(_), n, s) => format!("{} éléments indexés{}", fmt_count(n), if s { " · mise à jour…" } else { "" }),
                        };
                        ui.label(
                            RichText::new(format!("↑↓ choisir · Entrée ouvrir · Ctrl+Entrée nouvel onglet / ouvrir le fichier · Échap fermer   —   {idx_info}"))
                                .size(11.5)
                                .color(p.muted),
                        );
                    });
            });
        if let Some((path, ctrl, is_dir)) = go {
            self.goto = None;
            self.actions.push(match (is_dir, ctrl) {
                (true, false) => Action::Navigate(path),
                (true, true) => Action::NewTab(path),
                (false, false) => Action::ShowInFolder(path),
                (false, true) => Action::Open(path),
            });
        } else if close {
            self.goto = None;
        }
    }

    // ---------- Aperçu plein écran (Espace) ----------
    pub fn draw_fullview(&mut self, ui: &mut Ui) {
        let Some(path) = self.fullview.clone() else { return };
        let ctx = ui.ctx().clone();
        let screen = ctx.content_rect();
        let entry = self.tab().source().iter().find(|e| e.path == path).cloned().or_else(|| fsx::Entry::stat(&path));
        let Some(e) = entry else {
            self.fullview = None;
            return;
        };
        let (idx, n) = {
            let t = self.tab();
            (t.view_index_of(&path).map(|i| i + 1).unwrap_or(0), t.view.len())
        };
        let mut close = false;
        let mut open = false;
        let tex = if e.has_thumb() { self.thumbs.get(&e.path, 1600) } else { None };
        let text = if !e.is_dir && !e.has_thumb() && (fsx::is_text_ext(&e.ext) || e.size < 256 * 1024) {
            fsx::read_text_preview(&e.path, 64 * 1024)
        } else {
            None
        };
        egui::Area::new(egui::Id::new("fullview")).order(egui::Order::Foreground).fixed_pos(screen.min).show(&ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(8, 12, 20, 235));
            let content = r.shrink2(vec2(40.0, 70.0));
            if let Some(tex) = &tex {
                let sz = tex.size_vec2();
                let scale = (content.width() / sz.x).min(content.height() / sz.y).min(if sz.x.max(sz.y) <= 128.0 { 16.0 } else { 2.0 });
                let ir = Rect::from_center_size(content.center(), sz * scale);
                ui.painter().image(tex.id(), ir, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                if e.is_video() {
                    ui.painter().circle_filled(ir.center(), 40.0, Color32::from_black_alpha(150));
                    let s = 18.0;
                    let c = ir.center();
                    ui.painter().add(egui::Shape::convex_polygon(
                        vec![pos2(c.x - s * 0.7, c.y - s), pos2(c.x - s * 0.7, c.y + s), pos2(c.x + s, c.y)],
                        Color32::WHITE,
                        Stroke::NONE,
                    ));
                    let pr = ui.interact(ir, egui::Id::new("fv_play"), Sense::click());
                    if pr.clicked() {
                        open = true;
                    }
                }
            } else if let Some(t) = &text {
                let box_r = Rect::from_center_size(content.center(), vec2(content.width().min(1100.0), content.height()));
                ui.painter().rect_filled(box_r, CornerRadius::same(10), self.pal.code);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(box_r.shrink(16.0)));
                egui::ScrollArea::both().id_salt("fv_text").auto_shrink([false, false]).show(&mut child, |ui| {
                    ui.label(RichText::new(t).monospace().size(13.0).color(self.pal.text));
                });
            } else {
                let icon = crate::ui::entry_icon(&e);
                icons::paint(ui.painter(), Rect::from_center_size(content.center(), vec2(120.0, 120.0)), icon, Color32::from_gray(200));
                if e.has_thumb() {
                    ui.painter().text(content.center() + vec2(0.0, 90.0), Align2::CENTER_TOP, "chargement…", FontId::proportional(14.0), Color32::from_gray(160));
                }
            }
            // barre du haut : nom, position, aide
            let info = format!("{}   ·   {} / {}   ·   {}", e.name, idx, n, if e.is_dir { "Dossier".into() } else { fsx::human_size(e.size) });
            ui.painter().text(pos2(r.center().x, r.top() + 30.0), Align2::CENTER_CENTER, info, FontId::proportional(16.0), Color32::WHITE);
            ui.painter().text(
                pos2(r.center().x, r.bottom() - 28.0),
                Align2::CENTER_CENTER,
                "← → fichier précédent / suivant   ·   Espace ou Échap fermer   ·   double-clic ouvrir",
                FontId::proportional(12.5),
                Color32::from_gray(170),
            );
            if resp.double_clicked() {
                open = true;
            } else if resp.clicked() {
                close = true;
            }
        });
        if open {
            self.actions.push(Action::Open(e.path.clone()));
        }
        if close {
            self.fullview = None;
        }
    }

    // ---------- Renommage en lot ----------
    pub fn draw_batch(&mut self, ui: &mut Ui) {
        let Some(b) = self.batch.as_mut() else { return };
        let p = self.pal;
        let ctx = ui.ctx().clone();
        let names = batch_names(b);
        // conflits : deux fois le même nom, ou un fichier existant hors sélection
        let selected: std::collections::HashSet<&PathBuf> = b.paths.iter().collect();
        let mut seen = std::collections::HashSet::new();
        let conflicts: Vec<bool> = b
            .paths
            .iter()
            .zip(&names)
            .map(|(old, new)| match new {
                Ok(n) => {
                    let target = old.with_file_name(n);
                    let dup = !seen.insert(target.to_string_lossy().to_lowercase());
                    let exists = target.exists() && !selected.contains(&target);
                    n.is_empty() || dup || exists || n.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|'])
                }
                Err(_) => true,
            })
            .collect();
        let ok = !conflicts.iter().any(|c| *c);
        let changed = b.paths.iter().zip(&names).filter(|(o, n)| n.as_ref().ok().map(|n| Some(n.as_str()) != o.file_name().and_then(|f| f.to_str())).unwrap_or(false)).count();
        let mut apply = false;
        let mut close = false;
        egui::Window::new(format!("Renommer {} éléments", b.paths.len()))
            .collapsible(false)
            .resizable(true)
            .default_size(vec2(720.0, 520.0))
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(&ctx, |ui| {
                egui::Grid::new("batch_form").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                    ui.label("Nouveau nom");
                    let mut out = egui::TextEdit::singleline(&mut b.pattern).desired_width(420.0).font(FontId::monospace(13.0)).show(ui);
                    if b.fresh {
                        // tout sélectionner : taper remplace le motif
                        use egui::text::{CCursor, CCursorRange};
                        let n = b.pattern.chars().count();
                        out.state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(n))));
                        out.state.store(ui.ctx(), out.response.id);
                        out.response.request_focus();
                        b.fresh = false;
                    }
                    ui.end_row();
                    ui.label("");
                    ui.label(RichText::new("{nom} nom actuel · {n} numéro · {n:3} numéro sur 3 chiffres · {ext} extension · {date} date").size(11.5).color(p.muted));
                    ui.end_row();
                    ui.label("Chercher");
                    ui.add(egui::TextEdit::singleline(&mut b.find).desired_width(420.0).font(FontId::monospace(13.0)));
                    ui.end_row();
                    ui.label("Remplacer par");
                    ui.add(egui::TextEdit::singleline(&mut b.replace).desired_width(420.0).font(FontId::monospace(13.0)));
                    ui.end_row();
                    ui.label("");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut b.regex, "Expression régulière ($1, $2… dans le remplacement)");
                        ui.add_space(12.0);
                        ui.label("Numéro de départ");
                        ui.add(egui::DragValue::new(&mut b.start).range(0..=999_999));
                    });
                    ui.end_row();
                });
                ui.separator();
                egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
                    egui::Grid::new("batch_preview").num_columns(3).striped(true).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
                        for ((old, new), bad) in b.paths.iter().zip(&names).zip(&conflicts).take(500) {
                            let on = old.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                            ui.label(RichText::new(on).monospace().size(12.5).color(p.muted));
                            ui.label(RichText::new("→").color(p.muted));
                            match new {
                                Ok(n) => ui.label(RichText::new(n).monospace().size(12.5).color(if *bad { p.del } else { p.text })),
                                Err(e) => ui.label(RichText::new(e).size(12.0).color(p.del)),
                            };
                            ui.end_row();
                        }
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let label = format!("Renommer {changed} élément(s)");
                    let btn = egui::Button::new(RichText::new(label).color(p.on_accent).strong()).fill(p.accent).min_size(vec2(0.0, 34.0));
                    if ui.add_enabled(ok && changed > 0, btn).clicked() {
                        apply = true;
                    }
                    if ui.button("Annuler").clicked() {
                        close = true;
                    }
                    if !ok {
                        ui.label(RichText::new("Noms en rouge : en double, déjà pris ou invalides").color(p.del).size(12.0));
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
        if apply {
            let pairs: Vec<(PathBuf, PathBuf)> = b
                .paths
                .iter()
                .zip(names)
                .filter_map(|(o, n)| n.ok().map(|n| (o.clone(), o.with_file_name(n))))
                .filter(|(o, n)| o != n)
                .collect();
            self.batch = None;
            self.do_batch_rename(pairs);
        } else if close {
            self.batch = None;
        }
    }

    /// Renommage en lot dans la file d'opérations (deux temps, voir `ops::move_batch`).
    fn do_batch_rename(&mut self, pairs: Vec<(PathBuf, PathBuf)>) {
        self.ops.submit(crate::ops::Op::Rename(pairs));
    }
}

fn fmt_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{202F}');
        }
        out.push(c);
    }
    out
}

/// Calcule les nouveaux noms du renommage en lot.
pub fn batch_names(b: &crate::app::BatchRename) -> Vec<Result<String, String>> {
    let num_re = regex::Regex::new(r"\{n(?::(\d+))?\}").unwrap();
    let find_re = if b.regex && !b.find.is_empty() {
        match regex::Regex::new(&b.find) {
            Ok(r) => Some(Ok(r)),
            Err(e) => Some(Err(format!("expression invalide : {e}"))),
        }
    } else {
        None
    };
    b.paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let is_dir = p.is_dir();
            let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let (stem, ext) = if is_dir {
                (name.clone(), String::new())
            } else {
                match name.rfind('.') {
                    Some(k) if k > 0 => (name[..k].to_string(), name[k + 1..].to_string()),
                    _ => (name.clone(), String::new()),
                }
            };
            let date = std::fs::metadata(p)
                .and_then(|m| m.modified())
                .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d").to_string())
                .unwrap_or_default();
            let num = b.start as usize + i;
            let mut out = b.pattern.replace("{nom}", &stem).replace("{ext}", &ext).replace("{date}", &date);
            out = num_re
                .replace_all(&out, |c: &regex::Captures| match c.get(1).and_then(|w| w.as_str().parse::<usize>().ok()) {
                    Some(w) => format!("{num:0w$}"),
                    None => num.to_string(),
                })
                .into_owned();
            if !b.find.is_empty() {
                out = match &find_re {
                    Some(Ok(r)) => r.replace_all(&out, b.replace.as_str()).into_owned(),
                    Some(Err(e)) => return Err(e.clone()),
                    None => out.replace(&b.find, &b.replace),
                };
            }
            let out = out.trim().to_string();
            if !b.pattern.contains("{ext}") && !ext.is_empty() {
                Ok(format!("{out}.{ext}"))
            } else {
                Ok(out)
            }
        })
        .collect()
}

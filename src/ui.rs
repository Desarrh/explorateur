//! Dessin de l'interface.
use crate::app::{Act, Action, App, Confirm, DragPaths, Sort, MARK_SECS};
use crate::fsx::{self, Entry};
use crate::git::{self, LineKind};
use egui::{
    pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui,
};
use std::path::PathBuf;

const ROW_H: f32 = 28.0;

fn trunc(ui: &Ui, text: &str, font: FontId, color: Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(8.0));
    ui.painter().layout_job(job)
}

fn paint_text_left(ui: &Ui, rect: Rect, x: f32, text: &str, font: FontId, color: Color32, max_w: f32) {
    let g = trunc(ui, text, font, color, max_w);
    let y = rect.center().y - g.size().y / 2.0;
    ui.painter().galley(pos2(x, y), g, color);
}

fn paint_text_right(ui: &Ui, rect: Rect, right: f32, text: &str, font: FontId, color: Color32) {
    ui.painter().text(pos2(right, rect.center().y), Align2::RIGHT_CENTER, text, font, color);
}

fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn ago(secs: f32) -> String {
    let s = secs as u64;
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h", s / 3600)
    }
}

impl App {
    pub fn draw(&mut self, ui: &mut Ui) {
        let p = self.pal;
        self.draw_tabs(ui);
        self.draw_toolbar(ui);
        self.draw_status(ui);
        self.draw_sidebar(ui);
        if self.s.dev {
            self.draw_dev(ui);
        } else {
            if self.s.preview {
                egui::Panel::right("preview")
                    .resizable(true)
                    .default_size(320.0)
                    .min_size(220.0)
                    .frame(Frame::new().fill(p.side).inner_margin(Margin::same(16)))
                    .show(ui, |ui| self.draw_preview(ui));
            }
            egui::CentralPanel::default().frame(Frame::new().fill(p.surface)).show(ui, |ui| {
                match self.split_other {
                    Some(other) if other < self.tabs.len() => self.draw_split(ui, other),
                    _ => self.draw_files(ui),
                }
            });
        }
        self.draw_drag_ghost(ui);
        self.draw_confirm(ui);
        self.draw_help(ui);
        self.draw_batch(ui);
        self.draw_goto(ui);
        self.draw_fullview(ui);
    }

    /// Double volet : deux dossiers côte à côte. Le volet actif a un liseré bleu ;
    /// cliquer dans l'autre le rend actif (barre d'adresse, raccourcis, sélection).
    fn draw_split(&mut self, ui: &mut Ui, other: usize) {
        let p = self.pal;
        let (left, right) = if self.focus_right { (other, self.active) } else { (self.active, other) };
        let mut want_focus: Option<usize> = None;
        let full = ui.available_rect_before_wrap();
        let gap = 6.0;
        let half = (full.width() - gap) / 2.0;
        let rects = [
            Rect::from_min_size(full.min, vec2(half, full.height())),
            Rect::from_min_size(pos2(full.min.x + half + gap, full.min.y), vec2(half, full.height())),
        ];
        let saved = self.active;
        for (k, ti) in [left, right].into_iter().enumerate() {
            let rect = rects[k];
            let focused = ti == saved;
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("pane", k)));
            // en-tête du volet
            let head = Rect::from_min_size(rect.min, vec2(rect.width(), 30.0));
            child.painter().rect_filled(head, CornerRadius::ZERO, if focused { p.soft } else { p.side });
            child.painter().rect_filled(
                Rect::from_min_size(head.min, vec2(head.width(), 3.0)),
                CornerRadius::ZERO,
                if focused { p.accent } else { Color32::TRANSPARENT },
            );
            let title = self.tabs[ti].path.display().to_string();
            paint_text_left(&child, head, head.left() + 12.0, &title, FontId::proportional(12.5), if focused { p.text } else { p.muted }, head.width() - 24.0);
            child.advance_cursor_after_rect(head);
            if !focused && child.input(|i| i.pointer.any_pressed()) && child.rect_contains_pointer(rect) {
                want_focus = Some(ti);
            }
            self.active = ti;
            self.draw_files(&mut child);
            self.active = saved;
        }
        ui.painter().rect_filled(
            Rect::from_min_size(pos2(full.min.x + half, full.min.y), vec2(gap, full.height())),
            CornerRadius::ZERO,
            p.border,
        );
        ui.advance_cursor_after_rect(full);
        if let Some(ti) = want_focus {
            self.split_other = Some(self.active);
            self.active = ti;
            self.focus_right = !self.focus_right;
        }
    }

    // ---------- Onglets ----------
    fn draw_tabs(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::top("tabs")
            .show_separator_line(false)
            .frame(Frame::new().fill(p.chrome).inner_margin(Margin { left: 8, right: 8, top: 6, bottom: 0 }))
            .show(ui, |ui| {
                egui::ScrollArea::horizontal().id_salt("tabs_scroll").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    let font = FontId::proportional(15.0);
                    let can_close = self.tabs.len() > 1;
                    for (i, t) in self.tabs.iter().enumerate() {
                        let active = i == self.active;
                        let title = t.title();
                        let text_w = ui.painter().layout_no_wrap(title.clone(), font.clone(), p.text).size().x;
                        let w = (text_w + 92.0).clamp(150.0, 280.0);
                        let (rect, r) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
                        let hovered = r.hovered();
                        let fill = if active { p.surface } else if hovered { p.hover_bg } else { p.chrome };
                        ui.painter().rect_filled(rect, CornerRadius { nw: 10, ne: 10, sw: 0, se: 0 }, fill);
                        ui.painter().text(pos2(rect.left() + 14.0, rect.center().y), Align2::LEFT_CENTER, "📁", FontId::proportional(15.0), p.accent);
                        let mut text_max = rect.width() - 80.0;
                        if t.has_live() {
                            ui.painter().circle_filled(pos2(rect.right() - 44.0, rect.center().y), 4.0, p.live);
                            text_max -= 10.0;
                        }
                        paint_text_left(ui, rect, rect.left() + 40.0, &title, font.clone(), if active { p.text } else { p.muted }, text_max);
                        // Bouton fermer : enregistré APRÈS l'onglet pour être au-dessus et recevoir le clic.
                        if can_close {
                            let xr = Rect::from_center_size(pos2(rect.right() - 20.0, rect.center().y), vec2(26.0, 26.0));
                            let xresp = ui.interact(xr, ui.id().with(("tab_close", t.id)), Sense::click());
                            if xresp.hovered() {
                                ui.painter().rect_filled(xr, CornerRadius::same(6), p.soft);
                            }
                            let xc = if xresp.hovered() || active || hovered { p.text } else { p.muted };
                            ui.painter().text(xr.center(), Align2::CENTER_CENTER, "×", FontId::proportional(18.0), xc);
                            if xresp.on_hover_text("Fermer (Ctrl+W)").clicked() {
                                self.actions.push(Action::CloseTab(i));
                            }
                        }
                        if r.clicked() {
                            self.actions.push(Action::ActivateTab(i));
                        }
                        if r.middle_clicked() {
                            self.actions.push(Action::CloseTab(i));
                        }
                        if let Some(d) = r.dnd_release_payload::<DragPaths>() {
                            let copy = ui.input(|i| i.modifiers.ctrl);
                            self.actions.push(Action::DropInto { paths: d.0.clone(), dst: t.path.clone(), copy });
                        }
                        r.on_hover_text(t.path.display().to_string());
                    }
                    if ui
                        .add(egui::Button::new(RichText::new("+").size(20.0).color(p.muted)).frame(false).min_size(vec2(40.0, 40.0)))
                        .on_hover_text("Nouvel onglet (Ctrl+T)")
                        .clicked()
                    {
                        self.actions.push(Action::NewTab(PathBuf::new()));
                    }
                });
                });
            });
    }

    // ---------- Barre d'outils ----------
    fn draw_toolbar(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::top("toolbar")
            .frame(Frame::new().fill(p.surface).inner_margin(Margin::symmetric(12, 8)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let nav = |ui: &mut Ui, icon: &str, tip: &str, enabled: bool| {
                        ui.add_enabled(enabled, egui::Button::new(RichText::new(icon).size(15.0)).min_size(vec2(32.0, 32.0)))
                            .on_hover_text(tip)
                            .clicked()
                    };
                    let (can_back, can_fwd, can_up) =
                        (!self.tab().back.is_empty(), !self.tab().fwd.is_empty(), self.tab().path.parent().is_some());
                    if nav(ui, "←", "Précédent (Alt+←)", can_back) {
                        self.actions.push(Action::Back);
                    }
                    if nav(ui, "→", "Suivant (Alt+→)", can_fwd) {
                        self.actions.push(Action::Forward);
                    }
                    if nav(ui, "↑", "Dossier parent (Retour arrière)", can_up) {
                        self.actions.push(Action::Up);
                    }
                    if nav(ui, "⟳", "Actualiser (F5)", true) {
                        self.actions.push(Action::Refresh);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // De droite à gauche
                        let theme_icon = if self.s.dark { "☀" } else { "🌙" };
                        let tip = if self.s.dark { "Thème clair" } else { "Thème sombre" };
                        if ui.add(egui::Button::new(theme_icon).min_size(vec2(32.0, 32.0))).on_hover_text(tip).clicked() {
                            let d = !self.s.dark;
                            self.set_dark(d);
                        }
                        self.draw_more_menu(ui);
                        Frame::new().fill(p.soft).corner_radius(CornerRadius::same(9)).inner_margin(Margin::same(3)).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let seg = |ui: &mut Ui, on: bool, text: &str| {
                                let fill = if on { if p.dark { Color32::from_rgb(0x2B, 0x44, 0x70) } else { p.surface } } else { Color32::TRANSPARENT };
                                let color = if on { p.text } else { p.muted };
                                ui.add(egui::Button::new(RichText::new(text).color(color).strong()).fill(fill).min_size(vec2(0.0, 28.0)))
                            };
                            // ordre inversé (droite → gauche)
                            if seg(ui, self.s.dev, "</>  Dev").on_hover_text("Vue dev : changements live, diff, commit (Ctrl+Maj+D)").clicked() {
                                self.s.dev = true;
                            }
                            if seg(ui, !self.s.dev, "▦  Bureau").on_hover_text("Vue bureautique").clicked() {
                                self.s.dev = false;
                            }
                        });
                        if !self.s.dev {
                            ui.add_space(4.0);
                            let grid = self.s.grid;
                            if ui.add(egui::Button::new("▦").selected(grid).min_size(vec2(32.0, 32.0))).on_hover_text("Grille / miniatures (Ctrl+Maj+G)").clicked() {
                                self.s.grid = true;
                            }
                            if ui.add(egui::Button::new("☰").selected(!grid).min_size(vec2(32.0, 32.0))).on_hover_text("Liste détaillée (Ctrl+Maj+L)").clicked() {
                                self.s.grid = false;
                            }
                        }
                        ui.add_space(4.0);
                        let ltr = Layout::left_to_right(Align::Center);
                        ui.allocate_ui_with_layout(vec2(262.0, 34.0), ltr, |ui| self.draw_search(ui));
                        ui.add_space(4.0);
                        let w = ui.available_width();
                        ui.allocate_ui_with_layout(vec2(w, 34.0), ltr, |ui| self.draw_address(ui));
                    });
                });
            });
    }

    fn draw_more_menu(&mut self, ui: &mut Ui) {
        let cur = self.tab().path.clone();
        ui.menu_button(RichText::new("⋯").size(16.0), |ui| {
            ui.set_min_width(240.0);
            let mut hidden = self.s.show_hidden;
            if ui.checkbox(&mut hidden, "Afficher les fichiers cachés (Ctrl+H)").changed() {
                self.s.show_hidden = hidden;
                for t in &mut self.tabs {
                    t.view_dirty = true;
                }
            }
            ui.checkbox(&mut self.s.preview, "Volet d’aperçu");
            if ui.checkbox(&mut self.s.folder_sizes, "Taille des dossiers").changed() {
                for t in &mut self.tabs {
                    t.view_dirty = true;
                }
            }
            let mut split = self.split_other.is_some();
            if ui.checkbox(&mut split, "Double volet (F3)").changed() {
                self.actions.push(Action::ToggleSplit);
            }
            if ui.button("Aller à un dossier…   Ctrl+P").clicked() {
                self.actions.push(Action::OpenGoto);
                ui.close();
            }
            #[cfg(windows)]
            {
                let mut def = self.is_default;
                if ui
                    .checkbox(&mut def, "Explorateur par défaut (Win+E, dossiers)")
                    .on_hover_text("Ouvre les dossiers et Win+E dans cette app. Décocher rend la main à l’Explorateur Windows.")
                    .changed()
                {
                    self.set_default_explorer(def);
                }
            }
            ui.separator();
            if ui.button("Nouveau dossier (Ctrl+Maj+N)").clicked() {
                self.actions.push(Action::NewFolder);
                ui.close();
            }
            if ui.button("Nouveau fichier texte").clicked() {
                self.actions.push(Action::NewFile);
                ui.close();
            }
            ui.separator();
            if ui.button("Ouvrir un terminal ici").clicked() {
                self.actions.push(Action::Terminal(cur.clone()));
                ui.close();
            }
            if ui.button("Ouvrir dans VS Code").clicked() {
                self.actions.push(Action::Code(cur.clone()));
                ui.close();
            }
            if ui.button("Ouvrir dans l’Explorateur Windows").clicked() {
                self.actions.push(Action::Reveal(cur.clone()));
                ui.close();
            }
            if ui.button("Copier le chemin du dossier").clicked() {
                self.ctx.copy_text(cur.to_string_lossy().into_owned());
                ui.close();
            }
            ui.separator();
            let pinned = self.s.pinned.contains(&cur);
            if ui.button(if pinned { "Retirer des épingles" } else { "Épingler ce dossier" }).clicked() {
                self.actions.push(if pinned { Action::Unpin(cur.clone()) } else { Action::Pin(cur.clone()) });
                ui.close();
            }
            if ui.button("Raccourcis clavier (F1)").clicked() {
                self.show_help = true;
                ui.close();
            }
        });
    }

    fn draw_search(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let ai = self.active;
        let t = &mut self.tabs[ai];
        let rec_tip = "Chercher aussi dans les sous-dossiers";
        let rec = ui
            .add(egui::Button::new(RichText::new("⤵").size(14.0)).selected(t.recursive).min_size(vec2(30.0, 32.0)))
            .on_hover_text(rec_tip);
        if rec.clicked() {
            t.recursive = !t.recursive;
            self.actions.push(Action::StartSearch);
        }
        let hint = if t.recursive { "Rechercher partout dessous…" } else { "Filtrer (Ctrl+F)" };
        let edit = Frame::new()
            .fill(p.code)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("🔍").color(p.muted));
                    ui.add(
                        egui::TextEdit::singleline(&mut t.filter)
                            .id(egui::Id::new("search"))
                            .hint_text(hint)
                            .frame(Frame::NONE)
                            .desired_width(190.0),
                    )
                })
                .inner
            })
            .inner;
        if edit.changed() {
            t.view_dirty = true;
            if t.recursive {
                self.actions.push(Action::StartSearch);
            }
        }
        if edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            // Entrée : ouvrir le premier résultat
            if let Some(e) = t.entry(0) {
                self.actions.push(Action::Open(e.path.clone()));
            }
        }
    }

    fn draw_address(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let ai = self.active;
        let w = ui.available_width();
        let t = &mut self.tabs[ai];
        let frame = Frame::new()
            .fill(p.code)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 6));
        if let Some(text) = t.address.as_mut() {
            let mut go = None;
            let mut cancel = false;
            frame.show(ui, |ui| {
                ui.set_width(w - 22.0);
                let r = ui.add(egui::TextEdit::singleline(text).frame(Frame::NONE).desired_width(f32::INFINITY).id(egui::Id::new("address")));
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                }
                if r.lost_focus() {
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        go = Some(text.as_str().to_owned());
                    } else {
                        cancel = true;
                    }
                }
            });
            if let Some(g) = go {
                t.address = None;
                let g = g.trim().trim_matches('"').to_string();
                let path = if g == "~" { dirs::home_dir().unwrap_or_default() } else { PathBuf::from(g) };
                self.actions.push(Action::Navigate(path));
            } else if cancel {
                t.address = None;
            }
            return;
        }
        let path = t.path.clone();
        let resp = frame
            .show(ui, |ui| {
                ui.set_width(w - 22.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let mut parts: Vec<PathBuf> = path.ancestors().map(|a| a.to_path_buf()).collect();
                    parts.reverse();
                    let n = parts.len();
                    let skip = if n > 5 { n - 3 } else { 0 };
                    for (k, part) in parts.iter().enumerate() {
                        if skip > 0 && k > 0 && k < skip {
                            if k == 1 {
                                ui.label(RichText::new("… ›").color(p.muted));
                            }
                            continue;
                        }
                        let name = part
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| part.to_string_lossy().trim_end_matches('\\').to_string());
                        let last = k + 1 == n;
                        let color = if last { p.text } else { p.muted };
                        let b = ui.add(egui::Button::new(RichText::new(name).color(color)).frame(false));
                        if b.clicked() {
                            self.actions.push(Action::Navigate(part.clone()));
                        }
                        if b.middle_clicked() {
                            self.actions.push(Action::NewTab(part.clone()));
                        }
                        if let Some(d) = b.dnd_release_payload::<DragPaths>() {
                            let copy = ui.input(|i| i.modifiers.ctrl);
                            self.actions.push(Action::DropInto { paths: d.0.clone(), dst: part.clone(), copy });
                        }
                        if !last {
                            ui.label(RichText::new("›").color(p.muted));
                        }
                    }
                    // le reste de la barre : clic = édition du chemin
                    let rest = ui.available_rect_before_wrap();
                    let r = ui.interact(rest, ui.id().with("addr_rest"), Sense::click());
                    if r.clicked() {
                        self.tabs[ai].address = Some(path.to_string_lossy().into_owned());
                    }
                });
            })
            .response;
        let _ = resp;
    }

    // ---------- Barre latérale ----------
    fn draw_sidebar(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::left("side")
            .resizable(true)
            .default_size(220.0)
            .min_size(170.0)
            .frame(Frame::new().fill(p.side).inner_margin(Margin::symmetric(10, 14)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let cur = self.tab().path.clone();
                    let section = |ui: &mut Ui, t: &str| {
                        ui.add_space(6.0);
                        ui.label(RichText::new(t.to_uppercase()).size(11.0).color(p.muted).strong());
                        ui.add_space(2.0);
                    };
                    let item = |ui: &mut Ui, icon: &str, name: &str, path: &PathBuf, actions: &mut Vec<Action>, pinned: bool| {
                        let on = &cur == path;
                        let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                        let hover_drop = r.dnd_hover_payload::<DragPaths>().is_some();
                        let bg = if on {
                            p.soft
                        } else if hover_drop {
                            p.sel_bg
                        } else if r.hovered() {
                            p.hover_bg
                        } else {
                            Color32::TRANSPARENT
                        };
                        ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
                        ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, icon, FontId::proportional(14.0), p.accent);
                        paint_text_left(ui, rect, rect.left() + 34.0, name, FontId::proportional(13.5), p.text, rect.width() - 40.0);
                        if r.clicked() {
                            actions.push(Action::Navigate(path.clone()));
                        }
                        if r.middle_clicked() {
                            actions.push(Action::NewTab(path.clone()));
                        }
                        if let Some(d) = r.dnd_release_payload::<DragPaths>() {
                            let copy = ui.input(|i| i.modifiers.ctrl);
                            actions.push(Action::DropInto { paths: d.0.clone(), dst: path.clone(), copy });
                        }
                        let r = r.on_hover_text(path.display().to_string());
                        r.context_menu(|ui| {
                            if ui.button("Ouvrir dans un nouvel onglet").clicked() {
                                actions.push(Action::NewTab(path.clone()));
                                ui.close();
                            }
                            if ui.button("Ouvrir un terminal ici").clicked() {
                                actions.push(Action::Terminal(path.clone()));
                                ui.close();
                            }
                            if pinned && ui.button("Retirer des épingles").clicked() {
                                actions.push(Action::Unpin(path.clone()));
                                ui.close();
                            }
                            if !pinned && ui.button("Épingler").clicked() {
                                actions.push(Action::Pin(path.clone()));
                                ui.close();
                            }
                        });
                    };
                    let mut acts = Vec::new();
                    if self.s.dev {
                        if let Some(r) = &self.repo {
                            section(ui, "Dépôt");
                            let name = r.root.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                            item(ui, "⎇", &name, &r.root.clone(), &mut acts, false);
                        }
                    }
                    section(ui, "Accès rapide");
                    for (name, path) in self.quick.clone() {
                        item(ui, "📁", &name, &path, &mut acts, false);
                    }
                    if !self.s.pinned.is_empty() {
                        section(ui, "Épinglés");
                        for path in self.s.pinned.clone() {
                            let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
                            item(ui, "📌", &name, &path, &mut acts, true);
                        }
                    }
                    section(ui, "Disques");
                    for d in &self.drives {
                        item(ui, "🖴", &d.label, &d.root, &mut acts, false);
                        let used = 1.0 - d.free as f32 / d.total.max(1) as f32;
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
                        let bar = Rect::from_min_size(pos2(rect.left() + 10.0, rect.top() + 1.0), vec2(rect.width() - 20.0, 4.0));
                        ui.painter().rect_filled(bar, CornerRadius::same(2), p.soft);
                        let mut fill = bar;
                        fill.set_width(bar.width() * used);
                        ui.painter().rect_filled(fill, CornerRadius::same(2), if used > 0.9 { p.del } else { p.accent });
                        ui.label(
                            RichText::new(format!("   {} libres sur {}", fsx::human_size(d.free), fsx::human_size(d.total)))
                                .size(11.0)
                                .color(p.muted),
                        );
                    }
                    self.actions.extend(acts);
                });
            });
    }

    // ---------- Barre d'état ----------
    fn draw_status(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::bottom("status")
            .frame(Frame::new().fill(p.chrome).inner_margin(Margin::symmetric(14, 5)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let t = self.tab();
                    let n = t.view.len();
                    let sel = t.selection();
                    let mut txt = format!("{n} élément(s)");
                    if let Some(s) = &t.search {
                        txt = format!("{} résultat(s){}", s.results.len(), if s.running { " — recherche…" } else { "" });
                    }
                    if !sel.is_empty() {
                        let size: u64 = t.source().iter().filter(|e| t.selected.contains(&e.path)).map(|e| e.size).sum();
                        txt += &format!(" · {} sélectionné(s) · {}", sel.len(), fsx::human_size(size));
                    }
                    ui.label(RichText::new(txt).color(p.muted).size(12.0));
                    if self.watch_ok {
                        ui.label(RichText::new("● suivi live").color(p.live).size(12.0))
                            .on_hover_text("Les changements sur le disque apparaissent sans recharger la vue");
                    }
                    if let Some((label, done, total, cur)) = &self.progress {
                        let frac = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
                        ui.add(egui::ProgressBar::new(frac).desired_width(140.0).text(format!("{label} {done}/{total} — {cur}")));
                        if ui.small_button("Annuler").clicked() {
                            self.cancel_ops();
                        }
                    }
                    if let Some((msg, _, err)) = &self.toast {
                        ui.label(RichText::new(msg).color(if *err { p.del } else { p.text }).size(12.0));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let t = self.tab();
                        if t.loading {
                            ui.label(RichText::new("chargement…").color(p.muted).size(12.0));
                        } else {
                            ui.label(RichText::new(format!("ouvert en {:.0} ms", t.load_ms.max(1.0))).color(p.muted).size(12.0));
                        }
                    });
                });
            });
    }

    // ---------- Fichiers ----------
    fn draw_files(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let ai = self.active;
        // fond : clic = désélection, clic droit = menu du dossier, dépôt = déplacer ici
        let bg = ui.interact(ui.max_rect(), ui.id().with("files_bg"), Sense::click());
        if bg.clicked() {
            self.actions.push(Action::ClearSel);
        }
        let cur = self.tab().path.clone();
        if let Some(d) = bg.dnd_release_payload::<DragPaths>() {
            let copy = ui.input(|i| i.modifiers.ctrl);
            self.actions.push(Action::DropInto { paths: d.0.clone(), dst: cur.clone(), copy });
        }
        let mut acts = Vec::new();
        bg.context_menu(|ui| {
            if ui.button("Nouveau dossier").clicked() {
                acts.push(Action::NewFolder);
                ui.close();
            }
            if ui.button("Nouveau fichier texte").clicked() {
                acts.push(Action::NewFile);
                ui.close();
            }
            if ui.button("Coller").clicked() {
                acts.push(Action::Paste(cur.clone()));
                ui.close();
            }
            if ui.button("Annuler la dernière opération   Ctrl+Z").clicked() {
                acts.push(Action::Undo);
                ui.close();
            }
            ui.separator();
            if ui.button("Ouvrir Claude Code ici").clicked() {
                acts.push(Action::ClaudeHere(cur.clone()));
                ui.close();
            }
            if ui.button("Actualiser").clicked() {
                acts.push(Action::Refresh);
                ui.close();
            }
            if ui.button("Ouvrir un terminal ici").clicked() {
                acts.push(Action::Terminal(cur.clone()));
                ui.close();
            }
            if ui.button("Ouvrir dans VS Code").clicked() {
                acts.push(Action::Code(cur.clone()));
                ui.close();
            }
            if ui.button("Propriétés du dossier").clicked() {
                acts.push(Action::Properties(cur.clone()));
                ui.close();
            }
            if ui.button("Plus d’options (menu Windows)").clicked() {
                acts.push(Action::NativeMenu(vec![cur.clone()]));
                ui.close();
            }
        });
        self.actions.extend(acts);

        let t = &self.tabs[ai];
        if let Some(e) = &t.error {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Impossible d’ouvrir ce dossier").size(16.0).strong());
                ui.label(RichText::new(e).color(p.muted));
            });
            return;
        }
        if t.view.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                let msg = if t.loading {
                    "Chargement…"
                } else if !t.filter.is_empty() {
                    "Aucun résultat"
                } else {
                    "Ce dossier est vide"
                };
                ui.label(RichText::new(msg).color(p.muted).size(15.0));
            });
            return;
        }
        if self.s.grid {
            self.draw_grid(ui);
        } else {
            self.draw_list(ui);
        }
    }

    fn sort_header(&mut self, ui: &mut Ui, rect: Rect, label: &str, key: Sort, right: bool) {
        let p = self.pal;
        let r = ui.interact(rect, ui.id().with(("hdr", label)), Sense::click());
        let arrow = if self.s.sort == key { if self.s.asc { " ▲" } else { " ▼" } } else { "" };
        let txt = format!("{}{arrow}", label.to_uppercase());
        let color = if r.hovered() { p.text } else { p.muted };
        if right {
            paint_text_right(ui, rect, rect.right() - 4.0, &txt, FontId::proportional(11.0), color);
        } else {
            paint_text_left(ui, rect, rect.left(), &txt, FontId::proportional(11.0), color, rect.width());
        }
        if r.clicked() {
            self.actions.push(Action::Sort(key));
        }
    }

    fn draw_list(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let ai = self.active;
        let has_git = self.repo.as_ref().map(|r| r.loaded && self.tab().path.starts_with(&r.root)).unwrap_or(false);
        let searching = self.tab().search.is_some();
        // colonnes
        let full = ui.available_rect_before_wrap();
        // Colonnes adaptatives : en largeur réduite (double volet), on masque Type puis Modifié.
        let width = full.width();
        let w_kind = if width > 700.0 { 120.0 } else { 0.0 };
        let w_date = if width > 480.0 { 150.0 } else { 0.0 };
        let (w_size, w_git) = (86.0, if has_git { 34.0 } else { 0.0 });
        let x_size_r = full.right() - 16.0;
        let x_kind = x_size_r - w_size - w_kind;
        let x_date = x_kind - w_date - 8.0;
        let x_git = x_date - w_git;
        let name_w = (x_git - full.left() - 50.0).max(80.0);
        let header = Rect::from_min_size(full.min, vec2(full.width(), 32.0));
        ui.painter().line_segment([header.left_bottom(), header.right_bottom()], Stroke::new(1.0, p.border));
        let hy = |x0: f32, x1: f32| Rect::from_min_max(pos2(x0, header.top()), pos2(x1, header.bottom()));
        self.sort_header(ui, hy(full.left() + 16.0, x_git - 8.0), "Nom", Sort::Name, false);
        if w_date > 0.0 {
            self.sort_header(ui, hy(x_date, x_kind - 8.0), "Modifié", Sort::Modified, false);
        }
        if w_kind > 0.0 {
            self.sort_header(ui, hy(x_kind, x_size_r - w_size), "Type", Sort::Kind, false);
        }
        self.sort_header(ui, hy(x_size_r - w_size, x_size_r + 4.0), "Taille", Sort::Size, true);
        ui.advance_cursor_after_rect(header);

        let n = self.tabs[ai].view.len();
        let row_full = ROW_H + ui.spacing().item_spacing.y;
        let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("list", self.tabs[ai].id));
        if let Some(target) = self.tabs[ai].scroll_to.take() {
            let off = self.tabs[ai].scroll_offset;
            let vh = self.tabs[ai].view_h;
            let top = target as f32 * row_full;
            if top < off {
                sa = sa.vertical_scroll_offset(top);
            } else if top + row_full > off + vh {
                sa = sa.vertical_scroll_offset(top + row_full - vh);
            }
        }
        let mut acts: Vec<Action> = Vec::new();
        let out = sa.show_rows(ui, ROW_H, n, |ui, range| {
            for vi in range {
                let t = &self.tabs[ai];
                let Some(e) = t.entry(vi).cloned() else { continue };
                let e = &e;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
                let selected = t.selected.contains(&e.path);
                let mark = t.marks.get(&e.path).copied();
                let renaming = t.renaming.as_ref().map(|r| r.path == e.path).unwrap_or(false);
                // fond
                let drop_hover = e.is_dir && resp.dnd_hover_payload::<DragPaths>().is_some();
                let mut bg = if selected || drop_hover {
                    p.sel_bg
                } else if resp.hovered() {
                    p.hover_bg
                } else {
                    Color32::TRANSPARENT
                };
                let mut live_t = 0.0;
                if let Some(m) = mark {
                    live_t = 1.0 - (m.at.elapsed().as_secs_f32() / MARK_SECS);
                    if live_t > 0.0 && !selected {
                        let target = if m.kind == Act::Created { p.add_bg } else { p.live_bg };
                        bg = blend(if bg == Color32::TRANSPARENT { p.surface } else { bg }, target, live_t);
                    }
                }
                ui.painter().rect_filled(rect.shrink2(vec2(6.0, 0.0)), CornerRadius::same(6), bg);
                if live_t > 0.0 {
                    let c = if mark.map(|m| m.kind) == Some(Act::Created) { p.add } else { p.live };
                    let bar = Rect::from_min_size(pos2(rect.left() + 6.0, rect.top() + 4.0), vec2(3.0, rect.height() - 8.0));
                    ui.painter().rect_filled(bar, CornerRadius::same(1), c.gamma_multiply(live_t.max(0.2)));
                }
                // icône + nom
                let icon_color = if e.is_dir { p.accent } else { p.muted };
                ui.painter().text(pos2(rect.left() + 18.0, rect.center().y), Align2::LEFT_CENTER, fsx::icon_for(e), FontId::proportional(14.0), icon_color);
                let name = if searching { e.display.clone().unwrap_or_else(|| e.name.clone()) } else { e.name.clone() };
                let name_color = if e.hidden { p.muted } else { p.text };
                if !renaming {
                    paint_text_left(ui, rect, rect.left() + 42.0, &name, FontId::proportional(13.5), name_color, name_w);
                }
                // git
                if has_git {
                    if let Some(k) = self.git_kind(&e.path) {
                        let c = match k {
                            git::Kind::Untracked | git::Kind::Added => p.add,
                            git::Kind::Deleted => p.del,
                            _ => p.live,
                        };
                        ui.painter().text(pos2(x_git + 8.0, rect.center().y), Align2::LEFT_CENTER, k.letter(), FontId::monospace(12.5), c);
                    }
                }
                // date (« modifié il y a 3 s » quand c'est live)
                let (date, dc) = match mark {
                    Some(m) if live_t > 0.0 => (format!("{} · {}", m.kind.label(), ago(m.at.elapsed().as_secs_f32())), if m.kind == Act::Created { p.add } else { p.live }),
                    _ => (fsx::human_time(e.modified), p.muted),
                };
                if w_date > 0.0 {
                    paint_text_left(ui, rect, x_date, &date, FontId::proportional(12.5), dc, w_date);
                }
                if w_kind > 0.0 {
                    paint_text_left(ui, rect, x_kind, &fsx::kind_label(e), FontId::proportional(12.5), p.muted, w_kind - 8.0);
                }
                if !e.is_dir {
                    paint_text_right(ui, rect, x_size_r, &fsx::human_size(e.size), FontId::proportional(12.5), p.muted);
                } else if self.s.folder_sizes {
                    // taille du dossier, calculée en arrière-plan
                    let txt = match self.sizes.map.get(&e.path) {
                        Some(s) => fsx::human_size(*s),
                        None if self.sizes.is_pending(&e.path) => "…".into(),
                        None => String::new(),
                    };
                    paint_text_right(ui, rect, x_size_r, &txt, FontId::proportional(12.5), p.muted.gamma_multiply(0.85));
                }
                if renaming {
                    let r_rect = Rect::from_min_max(pos2(rect.left() + 38.0, rect.top() + 2.0), pos2(rect.left() + 42.0 + name_w, rect.bottom() - 2.0));
                    self.rename_box(ui, r_rect, &mut acts);
                }
                self.row_interactions(ui, &resp, e.clone(), selected, &mut acts);
            }
        });
        let t = &mut self.tabs[ai];
        t.scroll_offset = out.state.offset().y;
        t.view_h = out.inner_rect.height();
        self.actions.extend(acts);
    }

    fn draw_grid(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let ai = self.active;
        let (cell_w, cell_h, gap) = (156.0, 176.0, 12.0);
        let avail = ui.available_width() - 24.0;
        let cols = ((avail + gap) / (cell_w + gap)).floor().max(1.0) as usize;
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("grid_cols"), cols));
        let n = self.tabs[ai].view.len();
        let rows = n.div_ceil(cols);
        let row_full = cell_h + ui.spacing().item_spacing.y;
        let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("grid", self.tabs[ai].id));
        if let Some(target) = self.tabs[ai].scroll_to.take() {
            let row = target / cols;
            let off = self.tabs[ai].scroll_offset;
            let vh = self.tabs[ai].view_h;
            let top = row as f32 * row_full;
            if top < off {
                sa = sa.vertical_scroll_offset(top);
            } else if top + row_full > off + vh {
                sa = sa.vertical_scroll_offset(top + row_full - vh);
            }
        }
        let mut acts = Vec::new();
        let searching = self.tabs[ai].search.is_some();
        let out = sa.show_rows(ui, cell_h, rows, |ui, range| {
            for row in range {
                let (row_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), cell_h), Sense::hover());
                for c in 0..cols {
                    let vi = row * cols + c;
                    let t = &self.tabs[ai];
                    let Some(e) = t.entry(vi).cloned() else { break };
                    let rect = Rect::from_min_size(pos2(row_rect.left() + 12.0 + c as f32 * (cell_w + gap), row_rect.top() + 8.0), vec2(cell_w, cell_h - 12.0));
                    let resp = ui.interact(rect, ui.id().with(("cell", &e.path)), Sense::click_and_drag());
                    let selected = t.selected.contains(&e.path);
                    let mark = t.marks.get(&e.path).copied();
                    let renaming = t.renaming.as_ref().map(|r| r.path == e.path).unwrap_or(false);
                    let drop_hover = e.is_dir && resp.dnd_hover_payload::<DragPaths>().is_some();
                    let bg = if selected || drop_hover { p.sel_bg } else if resp.hovered() { p.hover_bg } else { Color32::TRANSPARENT };
                    ui.painter().rect_filled(rect, CornerRadius::same(12), bg);
                    if selected {
                        ui.painter().rect_stroke(rect, CornerRadius::same(12), Stroke::new(2.0, p.accent), StrokeKind::Inside);
                    }
                    if let Some(m) = mark {
                        let lt = 1.0 - m.at.elapsed().as_secs_f32() / MARK_SECS;
                        if lt > 0.0 {
                            let col = if m.kind == Act::Created { p.add } else { p.live };
                            ui.painter().rect_stroke(rect, CornerRadius::same(12), Stroke::new(2.0, col.gamma_multiply(lt)), StrokeKind::Inside);
                        }
                    }
                    let thumb = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(cell_w - 16.0, 112.0));
                    ui.painter().rect_filled(thumb, CornerRadius::same(8), p.soft);
                    let mut drawn = false;
                    if e.has_thumb() {
                        if let Some(tex) = self.thumbs.get(&e.path, 224) {
                            let sz = tex.size_vec2();
                            let scale = (thumb.width() / sz.x).min(thumb.height() / sz.y).min(if sz.x.max(sz.y) <= 128.0 { 8.0 } else { 1.0 });
                            let r = Rect::from_center_size(thumb.center(), sz * scale);
                            ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            drawn = true;
                        }
                        if e.is_video() {
                            play_badge(ui, thumb.center(), 18.0);
                        }
                    }
                    if !drawn {
                        let col = if e.is_dir { p.accent } else { p.muted };
                        ui.painter().text(thumb.center(), Align2::CENTER_CENTER, fsx::icon_for(&e), FontId::proportional(38.0), col);
                    }
                    let name = if searching { e.display.clone().unwrap_or_else(|| e.name.clone()) } else { e.name.clone() };
                    let name_rect = Rect::from_min_size(pos2(rect.left() + 8.0, thumb.bottom() + 6.0), vec2(cell_w - 16.0, 20.0));
                    if renaming {
                        self.rename_box(ui, name_rect, &mut acts);
                    } else {
                        paint_text_left(ui, name_rect, name_rect.left(), &name, FontId::proportional(13.0), p.text, name_rect.width());
                    }
                    let meta = if e.is_dir { "Dossier".to_string() } else { fsx::human_size(e.size) };
                    let meta_rect = name_rect.translate(vec2(0.0, 19.0));
                    paint_text_left(ui, meta_rect, meta_rect.left(), &meta, FontId::proportional(11.5), p.muted, meta_rect.width());
                    self.row_interactions(ui, &resp, e, selected, &mut acts);
                }
            }
        });
        let t = &mut self.tabs[ai];
        t.scroll_offset = out.state.offset().y;
        t.view_h = out.inner_rect.height();
        self.actions.extend(acts);
    }

    fn rename_box(&mut self, ui: &mut Ui, rect: Rect, acts: &mut Vec<Action>) {
        let ai = self.active;
        let Some(r) = self.tabs[ai].renaming.as_mut() else { return };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let mut out = egui::TextEdit::singleline(&mut r.text).desired_width(rect.width()).show(&mut child);
        if r.fresh {
            r.fresh = false;
            // Sélectionner le nom sans l'extension, comme l'Explorateur
            let is_dir = r.path.is_dir();
            let stem_len = if is_dir {
                r.text.chars().count()
            } else {
                match r.text.rfind('.') {
                    Some(i) if i > 0 => r.text[..i].chars().count(),
                    _ => r.text.chars().count(),
                }
            };
            use egui::text::{CCursor, CCursorRange};
            out.state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(stem_len))));
            out.state.store(ui.ctx(), out.response.id);
            out.response.request_focus();
        }
        if out.response.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                acts.push(Action::CancelRename);
            } else {
                acts.push(Action::CommitRename);
            }
        }
    }

    fn row_interactions(&mut self, ui: &mut Ui, resp: &egui::Response, e: Entry, selected: bool, acts: &mut Vec<Action>) {
        let mods = ui.input(|i| i.modifiers);
        if resp.clicked() {
            acts.push(Action::Select { path: e.path.clone(), ctrl: mods.command, shift: mods.shift });
        }
        if resp.double_clicked() {
            acts.push(Action::Open(e.path.clone()));
        }
        if resp.middle_clicked() && e.is_dir {
            acts.push(Action::NewTab(e.path.clone()));
        }
        if resp.secondary_clicked() && !selected {
            acts.push(Action::SelectOnly(e.path.clone()));
        }
        let targets = if selected { self.tab().selection() } else { vec![e.path.clone()] };
        // Maj+clic droit : directement le menu complet de Windows (comme dans l'Explorateur)
        if resp.secondary_clicked() && mods.shift {
            acts.push(Action::NativeMenu(targets.clone()));
        }
        if resp.drag_started() {
            let t = self.tab();
            let paths = if selected { t.selection() } else { vec![e.path.clone()] };
            resp.dnd_set_drag_payload(DragPaths(paths));
        }
        if e.is_dir {
            if let Some(d) = resp.dnd_release_payload::<DragPaths>() {
                acts.push(Action::DropInto { paths: d.0.clone(), dst: e.path.clone(), copy: mods.ctrl });
            }
        }
        let tab_path = self.tab().path.clone();
        let multi = self.tab().selected.len() > 1 && selected;
        let split = self.split_other.is_some();
        if mods.shift {
            return;
        }
        resp.context_menu(|ui| {
            ui.set_min_width(230.0);
            if ui.button("Ouvrir").clicked() {
                acts.push(Action::Open(e.path.clone()));
                ui.close();
            }
            if e.is_dir {
                if ui.button("Ouvrir dans un nouvel onglet").clicked() {
                    acts.push(Action::NewTab(e.path.clone()));
                    ui.close();
                }
                if ui.button("Ouvrir un terminal ici").clicked() {
                    acts.push(Action::Terminal(e.path.clone()));
                    ui.close();
                }
            } else if ui.button("Ouvrir avec…").clicked() {
                acts.push(Action::OpenWith(e.path.clone()));
                ui.close();
            }
            if ui.button("Ouvrir dans VS Code").clicked() {
                acts.push(Action::Code(e.path.clone()));
                ui.close();
            }
            if e.display.is_some() && ui.button("Afficher dans son dossier").clicked() {
                acts.push(Action::ShowInFolder(e.path.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button("Couper   Ctrl+X").clicked() {
                acts.push(Action::Copy(true));
                ui.close();
            }
            if ui.button("Copier   Ctrl+C").clicked() {
                acts.push(Action::Copy(false));
                ui.close();
            }
            if e.is_dir && ui.button("Coller dans ce dossier").clicked() {
                acts.push(Action::Paste(e.path.clone()));
                ui.close();
            }
            if ui.button("Copier le chemin   Ctrl+Maj+C").clicked() {
                acts.push(Action::CopyPath);
                ui.close();
            }
            if split {
                if ui.button("Copier vers l’autre volet   F6").clicked() {
                    acts.push(Action::ToOtherPane(false));
                    ui.close();
                }
                if ui.button("Déplacer vers l’autre volet   Maj+F6").clicked() {
                    acts.push(Action::ToOtherPane(true));
                    ui.close();
                }
            }
            ui.separator();
            if !multi && ui.button("Renommer   F2").clicked() {
                acts.push(Action::StartRename(e.path.clone()));
                ui.close();
            }
            if multi && ui.button("Renommer en lot…   F2").clicked() {
                acts.push(Action::OpenBatchRename);
                ui.close();
            }
            if ui.button("Compresser en .zip").clicked() {
                acts.push(Action::Zip(targets.clone()));
                ui.close();
            }
            if e.ext == "zip" && ui.button("Extraire ici").clicked() {
                acts.push(Action::Unzip(e.path.clone()));
                ui.close();
            }
            if ui.button("Mettre à la corbeille   Suppr").clicked() {
                acts.push(Action::Trash);
                ui.close();
            }
            if ui.button("Supprimer définitivement   Maj+Suppr").clicked() {
                acts.push(Action::AskDelete);
                ui.close();
            }
            ui.separator();
            if e.is_dir {
                if ui.button("Épingler").clicked() {
                    acts.push(Action::Pin(e.path.clone()));
                    ui.close();
                }
            }
            if ui.button("Afficher dans l’Explorateur Windows").clicked() {
                acts.push(Action::Reveal(e.path.clone()));
                ui.close();
            }
            if ui.button("Ouvrir Claude Code ici").clicked() {
                acts.push(Action::ClaudeHere(if e.is_dir { e.path.clone() } else { tab_path.clone() }));
                ui.close();
            }
            if ui.button("Propriétés").clicked() {
                acts.push(Action::Properties(e.path.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button("Plus d’options (menu Windows)   Maj+clic droit").clicked() {
                acts.push(Action::NativeMenu(targets.clone()));
                ui.close();
            }
        });
    }

    // ---------- Aperçu ----------
    fn draw_preview(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let t = self.tab();
        let sel = t.selection();
        let focus = if sel.len() == 1 {
            t.source().iter().find(|e| e.path == sel[0]).cloned()
        } else {
            None
        };
        if sel.len() > 1 {
            let size: u64 = t.source().iter().filter(|e| t.selected.contains(&e.path)).map(|e| e.size).sum();
            ui.label(RichText::new(format!("{} éléments sélectionnés", sel.len())).size(15.0).strong());
            ui.label(RichText::new(format!("Taille des fichiers : {}", fsx::human_size(size))).color(p.muted));
            return;
        }
        let Some(e) = focus else {
            ui.label(RichText::new(t.title()).size(15.0).strong());
            ui.label(RichText::new(format!("{} éléments", t.entries.len())).color(p.muted));
            ui.add_space(8.0);
            ui.label(RichText::new("Sélectionne un fichier pour l’aperçu.").color(p.muted));
            return;
        };
        let w = ui.available_width();
        let box_h = 220.0;
        let (rect, _) = ui.allocate_exact_size(vec2(w, box_h), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(12), p.soft);
        let mut drawn = false;
        if e.has_thumb() {
            if let Some(tex) = self.thumbs.get(&e.path, 640) {
                let sz = tex.size_vec2();
                let scale = (rect.width() / sz.x).min(rect.height() / sz.y).min(if sz.x.max(sz.y) <= 128.0 { 12.0 } else { 1.0 });
                let r = Rect::from_center_size(rect.center(), sz * scale);
                ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                drawn = true;
            }
            if e.is_video() {
                // Cliquer sur l'aperçu lance la vidéo dans le lecteur par défaut.
                play_badge(ui, rect.center(), 28.0);
                let r = ui.interact(rect, ui.id().with("preview_play"), Sense::click());
                if r.on_hover_text("Lire la vidéo").clicked() {
                    self.actions.push(Action::Open(e.path.clone()));
                }
                drawn = true;
            }
        }
        let text_preview = if !e.is_dir && !e.has_thumb() && (fsx::is_text_ext(&e.ext) || e.size < 64 * 1024) {
            let stale = self.preview_cache.as_ref().map(|c| c.0 != e.path || c.1 != e.modified).unwrap_or(true);
            if stale {
                self.preview_cache = Some((e.path.clone(), e.modified, fsx::read_text_preview(&e.path, 24 * 1024)));
            }
            self.preview_cache.as_ref().and_then(|c| c.2.clone())
        } else {
            None
        };
        if !drawn {
            if let Some(txt) = &text_preview {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(10.0)));
                egui::ScrollArea::both().id_salt("txtprev").auto_shrink([false, false]).show(&mut child, |ui| {
                    let lines: String = txt.lines().take(300).collect::<Vec<_>>().join("\n");
                    ui.label(RichText::new(lines).monospace().size(11.5).color(p.text));
                });
            } else {
                let col = if e.is_dir { p.accent } else { p.muted };
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, fsx::icon_for(&e), FontId::proportional(56.0), col);
            }
        }
        ui.add_space(12.0);
        ui.label(RichText::new(&e.name).size(15.0).strong());
        ui.add_space(6.0);
        egui::Grid::new("meta").num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
            ui.label(RichText::new("Type").color(p.muted));
            ui.label(fsx::kind_label(&e));
            ui.end_row();
            if !e.is_dir {
                ui.label(RichText::new("Taille").color(p.muted));
                ui.label(fsx::human_size(e.size));
                ui.end_row();
            } else if let Some(s) = self.sizes.map.get(&e.path) {
                ui.label(RichText::new("Taille").color(p.muted));
                ui.label(fsx::human_size(*s));
                ui.end_row();
            }
            ui.label(RichText::new("Modifié").color(p.muted));
            ui.label(fsx::human_time(e.modified));
            ui.end_row();
            if let Some(k) = self.git_kind(&e.path) {
                ui.label(RichText::new("Git").color(p.muted));
                ui.label(RichText::new(k.label()).color(p.live));
                ui.end_row();
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let open = egui::Button::new(RichText::new("Ouvrir").color(p.on_accent).strong()).fill(p.accent).min_size(vec2(100.0, 36.0));
            if ui.add(open).clicked() {
                self.actions.push(Action::Open(e.path.clone()));
            }
            if ui.add(egui::Button::new("Copier le chemin").min_size(vec2(0.0, 36.0))).clicked() {
                self.ctx.copy_text(e.path.to_string_lossy().into_owned());
                self.toast("Chemin copié", false);
            }
        });
    }

    // ---------- Vue Dev ----------
    fn draw_dev(&mut self, ui: &mut Ui) {
        let p = self.pal;
        if self.repo.is_some() {
            egui::Panel::right("diff")
                .resizable(true)
                .default_size(560.0)
                .min_size(320.0)
                .frame(Frame::new().fill(p.code))
                .show(ui, |ui| self.draw_diff_panel(ui));
        }
        egui::CentralPanel::default().frame(Frame::new().fill(p.surface)).show(ui, |ui| {
            egui::Panel::bottom("activity")
                .resizable(true)
                .default_size(260.0)
                .min_size(120.0)
                .frame(Frame::new().fill(p.surface).inner_margin(Margin::symmetric(16, 10)))
                .show(ui, |ui| self.draw_activity(ui));
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.surface).inner_margin(Margin::symmetric(0, 8)))
                .show(ui, |ui| self.draw_changes(ui));
        });
    }

    fn draw_changes(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let cur = self.tab().path.clone();
        let Some(repo) = self.repo.as_mut() else {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Ce dossier n’est pas suivi par git").size(16.0).strong());
                ui.label(RichText::new("Le suivi live des fichiers reste actif ci-dessous.").color(p.muted));
                ui.add_space(10.0);
                if ui.button("Initialiser un dépôt git ici").clicked() {
                    self.actions.push(Action::GitInit(cur.clone()));
                }
            });
            return;
        };
        let mut acts = Vec::new();
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(RichText::new(format!("Changements ({})", repo.changes.len())).size(15.0).strong());
            Frame::new().fill(p.soft).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
                ui.label(RichText::new(format!("⎇ {}", repo.branch)).monospace().size(12.0));
            });
            if !repo.ahead.is_empty() {
                ui.label(RichText::new(&repo.ahead).color(p.muted).size(12.0));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if ui.button("⟳").on_hover_text("Actualiser le statut git").clicked() {
                    acts.push(Action::GitRefresh);
                }
                let all = repo.excluded.is_empty();
                if ui.button(if all { "Tout décocher" } else { "Tout cocher" }).clicked() {
                    if all {
                        repo.excluded = repo.changes.iter().map(|c| c.rel.clone()).collect();
                    } else {
                        repo.excluded.clear();
                    }
                }
            });
        });
        ui.add_space(4.0);
        if repo.changes.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(if repo.loaded { "Rien à commit, l’arbre de travail est propre ✓" } else { "Lecture du statut git…" }).color(p.muted));
            });
            self.actions.extend(acts);
            return;
        }
        let root = repo.root.clone();
        let marks: Vec<(PathBuf, crate::app::Mark)> = self.tabs.iter().flat_map(|t| t.marks.iter().map(|(k, v)| (k.clone(), *v))).collect();
        let recent: std::collections::HashMap<PathBuf, f32> = self
            .activity
            .iter()
            .filter(|a| a.at.elapsed().as_secs_f32() < MARK_SECS)
            .map(|a| (a.path.clone(), 1.0 - a.at.elapsed().as_secs_f32() / MARK_SECS))
            .chain(marks.iter().map(|(k, m)| (k.clone(), 1.0 - m.at.elapsed().as_secs_f32() / MARK_SECS)))
            .collect();
        let repo = self.repo.as_mut().unwrap();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 34.0, repo.changes.len(), |ui, range| {
            for i in range {
                let ch = repo.changes[i].clone();
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                let sel = repo.selected.as_ref() == Some(&ch.rel);
                let abs = root.join(ch.rel.replace('/', "\\"));
                let live = recent.get(&abs).copied().unwrap_or(0.0);
                let mut bg = if sel { p.sel_bg } else if resp.hovered() { p.hover_bg } else { Color32::TRANSPARENT };
                if live > 0.0 && !sel {
                    bg = blend(p.surface, p.live_bg, live);
                }
                ui.painter().rect_filled(rect.shrink2(vec2(8.0, 1.0)), CornerRadius::same(6), bg);
                if live > 0.0 {
                    let bar = Rect::from_min_size(pos2(rect.left() + 8.0, rect.top() + 5.0), vec2(3.0, rect.height() - 10.0));
                    ui.painter().rect_filled(bar, CornerRadius::same(1), p.live);
                }
                // case à cocher
                let cb = Rect::from_center_size(pos2(rect.left() + 28.0, rect.center().y), vec2(16.0, 16.0));
                let cb_resp = ui.interact(cb.expand(4.0), ui.id().with(("cb", i)), Sense::click());
                let included = !repo.excluded.contains(&ch.rel);
                ui.painter().rect(cb, CornerRadius::same(4), if included { p.accent } else { p.surface }, Stroke::new(1.5, if included { p.accent } else { p.border }), StrokeKind::Inside);
                if included {
                    ui.painter().text(cb.center(), Align2::CENTER_CENTER, "✔", FontId::proportional(11.0), p.on_accent);
                }
                if cb_resp.clicked() {
                    if included {
                        repo.excluded.insert(ch.rel.clone());
                    } else {
                        repo.excluded.remove(&ch.rel);
                    }
                }
                let kc = match ch.kind {
                    git::Kind::Untracked | git::Kind::Added => p.add,
                    git::Kind::Deleted => p.del,
                    git::Kind::Conflict => p.del,
                    _ => p.live,
                };
                let right = rect.right() - 16.0;
                let stats_w = 90.0;
                let kind_w = 80.0;
                paint_text_left(ui, rect, rect.left() + 48.0, &ch.rel, FontId::monospace(12.5), p.text, right - stats_w - kind_w - rect.left() - 60.0);
                paint_text_left(ui, rect, right - stats_w - kind_w, ch.kind.label(), FontId::proportional(12.0), kc, kind_w);
                let plus = ch.plus.map(|v| format!("+{v}")).unwrap_or_default();
                let minus = ch.minus.filter(|&v| v > 0).map(|v| format!("−{v}")).unwrap_or_default();
                let mr = paint_galley_right(ui, rect, right, &minus, p.del);
                paint_galley_right(ui, rect, mr - 6.0, &plus, p.add);
                if resp.clicked() {
                    acts.push(Action::GitSelect(ch.rel.clone()));
                }
                if resp.double_clicked() {
                    acts.push(Action::Open(abs.clone()));
                }
                resp.context_menu(|ui| {
                    if ui.button("Ouvrir").clicked() {
                        acts.push(Action::Open(abs.clone()));
                        ui.close();
                    }
                    if ui.button("Ouvrir dans VS Code").clicked() {
                        acts.push(Action::Code(abs.clone()));
                        ui.close();
                    }
                    if ui.button("Afficher dans son dossier").clicked() {
                        acts.push(Action::ShowInFolder(abs.clone()));
                        ui.close();
                    }
                    if ui.button("Copier le chemin").clicked() {
                        ui.ctx().copy_text(abs.to_string_lossy().into_owned());
                        ui.close();
                    }
                    ui.separator();
                    let label = if ch.kind == git::Kind::Untracked { "Supprimer ce nouveau fichier (corbeille)" } else { "Annuler les changements (restaurer)" };
                    if ui.button(label).clicked() {
                        acts.push(Action::GitRestoreAsk(ch.rel.clone(), ch.kind == git::Kind::Untracked));
                        ui.close();
                    }
                });
            }
        });
        self.actions.extend(acts);
    }

    fn draw_activity(&mut self, ui: &mut Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            ui.label(RichText::new("●").color(if self.activity_paused { p.muted } else { p.live }));
            ui.label(RichText::new("Activité en direct").size(14.0).strong());
            ui.label(RichText::new("sans recharger la vue").color(p.muted).size(12.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Effacer").clicked() {
                    self.activity.clear();
                }
                let label = if self.activity_paused { "Reprendre" } else { "Pause" };
                if ui.button(label).clicked() {
                    self.activity_paused = !self.activity_paused;
                }
            });
        });
        ui.add_space(4.0);
        if self.activity.is_empty() {
            ui.label(RichText::new("Aucun changement pour l’instant. Les fichiers modifiés par Claude ou un autre outil s’afficheront ici en temps réel.").color(p.muted));
            return;
        }
        let root = self.repo.as_ref().map(|r| r.root.clone());
        let mut acts = Vec::new();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 30.0, self.activity.len(), |ui, range| {
            for i in range {
                let a = &self.activity[i];
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                let bg = if resp.hovered() { p.hover_bg } else { p.soft };
                ui.painter().rect_filled(rect.shrink2(vec2(0.0, 2.0)), CornerRadius::same(6), bg);
                let col = match a.kind {
                    Act::Created => p.add,
                    Act::Removed => p.del,
                    Act::Modified => p.live,
                };
                paint_text_left(ui, rect, rect.left() + 10.0, &format!("il y a {}", ago(a.at.elapsed().as_secs_f32())), FontId::proportional(11.5), p.muted, 90.0);
                paint_text_left(ui, rect, rect.left() + 100.0, &a.label, FontId::monospace(12.0), p.text, rect.width() - 230.0);
                let what = if a.count > 1 { format!("{} ×{}", a.kind.label(), a.count) } else { a.kind.label().to_string() };
                paint_text_right(ui, rect, rect.right() - 10.0, &what, FontId::proportional(12.0), col);
                if resp.clicked() {
                    if let Some(r) = &root {
                        if let Ok(rel) = a.path.strip_prefix(r) {
                            acts.push(Action::GitSelect(rel.to_string_lossy().replace('\\', "/")));
                        }
                    }
                }
                if resp.double_clicked() && a.kind != Act::Removed {
                    acts.push(Action::ShowInFolder(a.path.clone()));
                }
                resp.on_hover_text(format!("{}\n{}", a.path.display(), fsx::human_time(Some(a.wall))));
            }
        });
        self.actions.extend(acts);
    }

    fn draw_diff_panel(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let mut acts = Vec::new();
        let Some(repo) = self.repo.as_mut() else { return };
        // Zone de commit (en bas)
        egui::Panel::bottom("commit")
            .frame(Frame::new().fill(p.side).inner_margin(Margin::same(14)))
            .show(ui, |ui| {
                let n = repo.included().len();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Commit").strong());
                    ui.label(RichText::new(format!("sur {}", repo.branch)).color(p.muted).monospace().size(12.0));
                });
                ui.add(
                    egui::TextEdit::multiline(&mut repo.msg)
                        .hint_text("Message de commit (Ctrl+Entrée pour valider)")
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                let ctrl_enter = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    let can = n > 0 && !repo.msg.trim().is_empty() && repo.busy.is_none();
                    let label = repo.busy.unwrap_or("");
                    let btn = egui::Button::new(RichText::new(if label.is_empty() { format!("Commit {n} fichier(s)") } else { label.to_string() }).color(p.on_accent).strong())
                        .fill(p.accent)
                        .min_size(vec2(160.0, 36.0));
                    if ui.add_enabled(can, btn).clicked() || (can && ctrl_enter) {
                        acts.push(Action::GitCommit);
                    }
                    let push_label = if repo.ahead.contains("avance") { format!("Push ({})", repo.ahead) } else { "Push".into() };
                    if ui.add_enabled(repo.busy.is_none(), egui::Button::new(push_label).min_size(vec2(0.0, 36.0))).clicked() {
                        acts.push(Action::GitPush);
                    }
                });
                if let Some((m, ok)) = &repo.last {
                    ui.label(RichText::new(m).size(12.0).color(if *ok { p.add } else { p.del }));
                }
            });
        // En-tête du fichier
        let sel = repo.selected.clone();
        egui::Panel::top("diff_head")
            .frame(Frame::new().fill(p.code).inner_margin(Margin::symmetric(14, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    match &sel {
                        Some(rel) => {
                            ui.label(RichText::new(rel).monospace().strong());
                            if let Some(c) = repo.changes.iter().find(|c| &c.rel == rel) {
                                ui.label(RichText::new(c.kind.label()).color(p.live).size(12.0));
                                let untracked = c.kind == git::Kind::Untracked;
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.button("Restaurer").on_hover_text("Annuler les changements de ce fichier").clicked() {
                                        acts.push(Action::GitRestoreAsk(rel.clone(), untracked));
                                    }
                                    if ui.button("Ouvrir").clicked() {
                                        acts.push(Action::Open(repo.root.join(rel.replace('/', "\\"))));
                                    }
                                });
                            }
                        }
                        None => {
                            ui.label(RichText::new("Sélectionne un changement pour voir le diff").color(p.muted));
                        }
                    }
                });
            });
        // Le diff
        egui::CentralPanel::default().frame(Frame::new().fill(p.code)).show(ui, |ui| {
            if repo.diff_for.is_none() || repo.diff_for != sel {
                if sel.is_some() {
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new("Chargement du diff…").color(p.muted)));
                }
                return;
            }
            if repo.diff.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| ui.label(RichText::new("Pas de différence texte (fichier binaire ou vide)").color(p.muted)));
                return;
            }
            let line_h = 19.0;
            let font = FontId::monospace(12.5);
            let max_chars = repo.diff.iter().map(|l| l.text.len()).max().unwrap_or(40);
            let width = (max_chars as f32 * 7.4 + 110.0).max(ui.available_width());
            egui::ScrollArea::both().auto_shrink([false, false]).id_salt("diff_scroll").show_rows(ui, line_h, repo.diff.len(), |ui, range| {
                ui.set_min_width(width);
                for i in range {
                    let l = &repo.diff[i];
                    let (rect, _) = ui.allocate_exact_size(vec2(width, line_h), Sense::hover());
                    let (bg, fg, sign) = match l.kind {
                        LineKind::Add => (p.add_bg, p.add, "+"),
                        LineKind::Del => (p.del_bg, p.del, "−"),
                        LineKind::Hunk => (p.soft, p.muted, ""),
                        LineKind::Ctx => (Color32::TRANSPARENT, p.text, ""),
                    };
                    ui.painter().rect_filled(rect, CornerRadius::ZERO, bg);
                    let no = l.new_no.or(l.old_no).map(|n| n.to_string()).unwrap_or_default();
                    ui.painter().text(pos2(rect.left() + 44.0, rect.center().y), Align2::RIGHT_CENTER, no, font.clone(), p.muted);
                    ui.painter().text(pos2(rect.left() + 56.0, rect.center().y), Align2::LEFT_CENTER, sign, font.clone(), fg);
                    let text = l.text.replace('\t', "    ");
                    ui.painter().text(pos2(rect.left() + 72.0, rect.center().y), Align2::LEFT_CENTER, text, font.clone(), fg);
                }
            });
        });
        self.actions.extend(acts);
    }

    // ---------- Fenêtres ----------
    fn draw_drag_ghost(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if let Some(payload) = egui::DragAndDrop::payload::<DragPaths>(&ctx) {
            if let Some(pos) = ctx.pointer_interact_pos() {
                let p = self.pal;
                let copy = ctx.input(|i| i.modifiers.ctrl);
                egui::Area::new(egui::Id::new("drag_ghost"))
                    .fixed_pos(pos + vec2(14.0, 10.0))
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .show(&ctx, |ui| {
                        Frame::new().fill(p.accent).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(10, 5)).show(ui, |ui| {
                            let verb = if copy { "Copier" } else { "Déplacer" };
                            ui.label(RichText::new(format!("{verb} {} élément(s)", payload.0.len())).color(p.on_accent));
                        });
                    });
            }
        }
    }

    fn draw_confirm(&mut self, ui: &mut Ui) {
        let Some(c) = &self.confirm else { return };
        let p = self.pal;
        let (title, body, yes) = match c {
            Confirm::Delete(paths) => (
                "Supprimer définitivement ?",
                format!("{} élément(s) seront supprimés sans passer par la corbeille. C’est irréversible.", paths.len()),
                "Supprimer",
            ),
            Confirm::Restore { rel, untracked } => (
                "Annuler les changements ?",
                if *untracked {
                    format!("Le nouveau fichier « {rel} » sera mis à la corbeille.")
                } else {
                    format!("« {rel} » reviendra à sa version du dernier commit. Les modifications non commitées seront perdues.")
                },
                "Restaurer",
            ),
        };
        let mut answer = None;
        let ctx = ui.ctx().clone();
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(&ctx, |ui| {
                ui.set_max_width(380.0);
                ui.label(body);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new(yes).color(Color32::WHITE).strong()).fill(p.del)).clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Annuler").clicked() {
                        answer = Some(false);
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    answer = Some(false);
                }
            });
        match answer {
            Some(true) => self.confirm_yes(),
            Some(false) => self.confirm = None,
            None => {}
        }
    }

    fn draw_help(&mut self, ui: &mut Ui) {
        if !self.show_help {
            return;
        }
        let ctx = ui.ctx().clone();
        let mut open = true;
        egui::Window::new("Raccourcis clavier").open(&mut open).collapsible(false).resizable(false).show(&ctx, |ui| {
            egui::Grid::new("keys").num_columns(2).spacing(vec2(24.0, 4.0)).show(ui, |ui| {
                for (k, v) in [
                    ("Ctrl+T / Ctrl+W", "Nouvel onglet / fermer"),
                    ("Ctrl+Tab", "Onglet suivant"),
                    ("Ctrl+L", "Modifier le chemin"),
                    ("Ctrl+F", "Filtrer / rechercher"),
                    ("Alt+← / Alt+→ / Retour", "Précédent / suivant / parent"),
                    ("Entrée", "Ouvrir"),
                    ("F2", "Renommer"),
                    ("Suppr / Maj+Suppr", "Corbeille / suppression définitive"),
                    ("Ctrl+C / X / V", "Copier / couper / coller (compatible Explorateur)"),
                    ("Ctrl+Maj+C", "Copier le chemin"),
                    ("Ctrl+Maj+N", "Nouveau dossier"),
                    ("Ctrl+A", "Tout sélectionner"),
                    ("Ctrl+H", "Fichiers cachés"),
                    ("Ctrl+Maj+L / Ctrl+Maj+G", "Liste / grille"),
                    ("Ctrl+Maj+D", "Basculer Bureau / Dev"),
                    ("Lettres", "Aller au fichier qui commence par…"),
                    ("Glisser (+Ctrl)", "Déplacer (copier) vers un dossier"),
                    ("Clic molette", "Ouvrir un dossier dans un onglet"),
                    ("F5", "Actualiser"),
                    ("Ctrl+P", "Aller à un dossier (fréquents et récents)"),
                    ("F3", "Double volet"),
                    ("F6 / Maj+F6", "Copier / déplacer vers l’autre volet"),
                    ("Ctrl+Z", "Annuler la dernière opération"),
                    ("Espace", "Aperçu plein écran (← → pour naviguer)"),
                    ("F2 (plusieurs fichiers)", "Renommage en lot"),
                    ("Maj+clic droit", "Menu complet de Windows"),
                ] {
                    ui.label(RichText::new(k).monospace().strong());
                    ui.label(v);
                    ui.end_row();
                }
            });
        });
        if !open {
            self.show_help = false;
        }
    }
}

fn play_badge(ui: &Ui, c: egui::Pos2, r: f32) {
    ui.painter().circle_filled(c, r, Color32::from_black_alpha(150));
    let s = r * 0.45;
    let pts = vec![pos2(c.x - s * 0.7, c.y - s), pos2(c.x - s * 0.7, c.y + s), pos2(c.x + s, c.y)];
    ui.painter().add(egui::Shape::convex_polygon(pts, Color32::WHITE, Stroke::NONE));
}

fn paint_galley_right(ui: &Ui, rect: Rect, right: f32, text: &str, color: Color32) -> f32 {
    if text.is_empty() {
        return right;
    }
    let g = ui.painter().layout_no_wrap(text.to_string(), FontId::monospace(12.0), color);
    let w = g.size().x;
    let y = rect.center().y - g.size().y / 2.0;
    ui.painter().galley(pos2(right - w, y), g, color);
    right - w
}

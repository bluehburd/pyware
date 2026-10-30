#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{
    self, pos2,
    text::{CCursor, CCursorRange, LayoutJob, TextFormat},
    vec2, Align, Align2, Color32, FontFamily, FontId, Id, Key, Modifiers, PointerButton, Pos2,
    Rect, Sense, Stroke, Vec2,
};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

const NOTE: Color32 = Color32::from_rgb(0xfe, 0xff, 0x9c);
const INK: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1a);
const BORDER: Color32 = Color32::from_rgb(0xd6, 0xd6, 0x8a);
const BG: Color32 = Color32::from_rgb(0xf7, 0xf7, 0xf9);
const DOT: Color32 = Color32::from_rgb(0xc4, 0xc4, 0xc8);
const ACC: Color32 = Color32::from_rgb(0x5b, 0x5b, 0xd6);
const FS: f32 = 16.0;
const HEAD: f32 = 26.0;
const PALETTE: [[u8; 3]; 7] = [
    [0x1a, 0x1a, 0x1a],
    [0xd1, 0x34, 0x38],
    [0xca, 0x50, 0x10],
    [0x10, 0x7c, 0x10],
    [0x00, 0x67, 0xc0],
    [0x87, 0x64, 0xb8],
    [0x60, 0x5e, 0x5c],
];

// ---------- rich text model: text + runs of equal style (lengths in chars) ----------

#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
struct Style {
    b: bool,
    i: bool,
    u: bool,
    s: bool,
    color: Option<[u8; 3]>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Run {
    len: usize,
    st: Style,
}

#[derive(Clone, Serialize, Deserialize)]
struct Note {
    id: u64,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    text: String,
    runs: Vec<Run>,
    #[serde(skip)]
    sel: (usize, usize),
    #[serde(skip)]
    typing: Option<(usize, Style)>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Tab {
    name: String,
    notes: Vec<Note>,
    pan: [f32; 2],
    zoom: f32,
}

#[derive(Serialize, Deserialize, Default)]
struct Data {
    tabs: Vec<Tab>,
    cur: usize,
    next_id: u64,
}

fn normalize(runs: &mut Vec<Run>) {
    let mut out: Vec<Run> = Vec::new();
    for r in runs.drain(..) {
        if r.len == 0 {
            continue;
        }
        if let Some(l) = out.last_mut() {
            if l.st == r.st {
                l.len += r.len;
                continue;
            }
        }
        out.push(r);
    }
    *runs = out;
}

// style of the character just before `pos` (what typing there continues)
fn style_at(runs: &[Run], pos: usize) -> Style {
    let mut acc = 0;
    let mut last = Style::default();
    for r in runs {
        if pos > acc && pos <= acc + r.len {
            return r.st;
        }
        acc += r.len;
        last = r.st;
    }
    if pos == 0 {
        runs.first().map(|r| r.st).unwrap_or_default()
    } else {
        last
    }
}

fn split_at(runs: &mut Vec<Run>, pos: usize) {
    let mut acc = 0;
    for i in 0..runs.len() {
        let l = runs[i].len;
        if pos > acc && pos < acc + l {
            let st = runs[i].st;
            runs[i].len = pos - acc;
            runs.insert(
                i + 1,
                Run {
                    len: acc + l - pos,
                    st,
                },
            );
            return;
        }
        acc += l;
    }
}

fn set_style(runs: &mut Vec<Run>, a: usize, b: usize, f: &dyn Fn(&mut Style)) {
    if a >= b {
        return;
    }
    split_at(runs, a);
    split_at(runs, b);
    let mut acc = 0;
    for r in runs.iter_mut() {
        if acc >= a && acc + r.len <= b {
            f(&mut r.st);
        }
        acc += r.len;
    }
    normalize(runs);
}

fn range_all(runs: &[Run], a: usize, b: usize, get: &dyn Fn(&Style) -> bool) -> bool {
    let mut acc = 0;
    for r in runs {
        let (s, e) = (acc, acc + r.len);
        acc = e;
        if e > a && s < b && !get(&r.st) {
            return false;
        }
    }
    true
}

// the text changed: find the edit by common prefix/suffix and keep run styles in step
fn reconcile(note: &mut Note, old: &str) {
    let o: Vec<char> = old.chars().collect();
    let n: Vec<char> = note.text.chars().collect();
    let mut p = 0;
    while p < o.len() && p < n.len() && o[p] == n[p] {
        p += 1;
    }
    let mut q = 0;
    while q < o.len() - p && q < n.len() - p && o[o.len() - 1 - q] == n[n.len() - 1 - q] {
        q += 1;
    }
    let removed = o.len() - p - q;
    let ins = n.len() - p - q;
    let mut st = style_at(&note.runs, p);
    if let Some((tp, ts)) = note.typing {
        if tp == p {
            st = ts;
        }
    }
    note.typing = None;
    let mut out: Vec<Run> = Vec::new();
    let mut acc = 0;
    let mut done = false;
    for r in note.runs.iter() {
        let (s, e) = (acc, acc + r.len);
        acc = e;
        if s < p {
            out.push(Run {
                len: e.min(p) - s,
                st: r.st,
            });
        }
        if !done && e >= p {
            if ins > 0 {
                out.push(Run { len: ins, st });
            }
            done = true;
        }
        let rs = s.max(p + removed);
        if e > rs {
            out.push(Run {
                len: e - rs,
                st: r.st,
            });
        }
    }
    if !done && ins > 0 {
        out.push(Run { len: ins, st });
    }
    normalize(&mut out);
    let total: usize = out.iter().map(|r| r.len).sum();
    note.runs = if total == n.len() {
        out
    } else {
        vec![Run {
            len: n.len(),
            st: Style::default(),
        }]
    };
}

fn toggle(note: &mut Note, which: u8) {
    let (a, b) = note.sel;
    let get = |s: &Style| match which {
        0 => s.b,
        1 => s.i,
        2 => s.u,
        _ => s.s,
    };
    let setf = |s: &mut Style, v: bool| match which {
        0 => s.b = v,
        1 => s.i = v,
        2 => s.u = v,
        _ => s.s = v,
    };
    if a < b {
        let all = range_all(&note.runs, a, b, &get);
        set_style(&mut note.runs, a, b, &|s| setf(s, !all));
    } else {
        let mut s = match note.typing {
            Some((tp, ts)) if tp == a => ts,
            _ => style_at(&note.runs, a),
        };
        let nv = !get(&s);
        setf(&mut s, nv);
        note.typing = Some((a, s));
    }
}

fn set_color(note: &mut Note, c: Option<[u8; 3]>) {
    let (a, b) = note.sel;
    if a < b {
        set_style(&mut note.runs, a, b, &|s| s.color = c);
    } else {
        let mut s = match note.typing {
            Some((tp, ts)) if tp == a => ts,
            _ => style_at(&note.runs, a),
        };
        s.color = c;
        note.typing = Some((a, s));
    }
}

fn build_job(text: &str, runs: &[Run], size: f32, wrap: f32, bold: &FontFamily) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let idx: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let n = idx.len() - 1;
    let fmt = |st: &Style| {
        let c = st
            .color
            .map(|c| Color32::from_rgb(c[0], c[1], c[2]))
            .unwrap_or(INK);
        TextFormat {
            font_id: FontId::new(
                size,
                if st.b {
                    bold.clone()
                } else {
                    FontFamily::Proportional
                },
            ),
            color: c,
            italics: st.i,
            underline: if st.u {
                Stroke::new(1.0, c)
            } else {
                Stroke::NONE
            },
            strikethrough: if st.s {
                Stroke::new(1.0, c)
            } else {
                Stroke::NONE
            },
            ..Default::default()
        }
    };
    let total: usize = runs.iter().map(|r| r.len).sum();
    if total != n || runs.is_empty() {
        job.append(text, 0.0, fmt(&Style::default()));
        return job;
    }
    let mut pos = 0;
    for r in runs {
        job.append(&text[idx[pos]..idx[pos + r.len]], 0.0, fmt(&r.st));
        pos += r.len;
    }
    job
}

// right-click menu; returns true if the formatting changed
fn format_menu(ui: &mut egui::Ui, note: &mut Note) -> bool {
    let mut ch = false;
    let cur = if note.sel.0 < note.sel.1 {
        style_at(&note.runs, note.sel.0 + 1)
    } else {
        match note.typing {
            Some((tp, ts)) if tp == note.sel.0 => ts,
            _ => style_at(&note.runs, note.sel.0),
        }
    };
    ui.horizontal(|ui| {
        for (label, which, on) in [
            ("B", 0u8, cur.b),
            ("I", 1, cur.i),
            ("U", 2, cur.u),
            ("S", 3, cur.s),
        ] {
            let mut t = egui::RichText::new(label);
            t = match which {
                0 => t.strong(),
                1 => t.italics(),
                2 => t.underline(),
                _ => t.strikethrough(),
            };
            if ui.add(egui::Button::new(t).selected(on)).clicked() {
                toggle(note, which);
                ch = true;
                ui.close_menu();
            }
        }
    });
    ui.horizontal(|ui| {
        for c in PALETTE {
            let (rect, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
            ui.painter()
                .circle_filled(rect.center(), 8.0, Color32::from_rgb(c[0], c[1], c[2]));
            if resp.clicked() {
                set_color(note, Some(c));
                ch = true;
                ui.close_menu();
            }
        }
    });
    if ui.button("Clear formatting").clicked() {
        let (a, b) = note.sel;
        if a < b {
            set_style(&mut note.runs, a, b, &|s| *s = Style::default());
        } else {
            note.typing = Some((a, Style::default()));
        }
        ch = true;
        ui.close_menu();
    }
    ch
}

// ---------- board ----------

struct View {
    origin: Pos2,
    pan: Vec2,
    z: f32,
}

struct NoteOut {
    delete: bool,
    front: bool,
    changed: bool,
}

fn note_ui(
    ui: &mut egui::Ui,
    note: &mut Note,
    v: &View,
    selected: bool,
    bold: &FontFamily,
) -> NoteOut {
    let z = v.z;
    let id = note.id;
    let mut out = NoteOut {
        delete: false,
        front: false,
        changed: false,
    };
    let r = Rect::from_min_size(
        v.origin + v.pan + vec2(note.x, note.y) * z,
        vec2(note.w, note.h) * z,
    );
    if !r.intersects(ui.clip_rect()) {
        return out;
    }
    let hh = HEAD * z;
    let p = ui.painter().clone();
    if selected {
        p.rect_filled(r.expand(2.0), 0.0, ACC);
    }
    p.rect_filled(
        r.translate(vec2(1.0, 2.0)),
        0.0,
        Color32::from_black_alpha(36),
    );
    p.rect_filled(r, 0.0, BORDER);
    p.rect_filled(r.shrink(1.0), 0.0, NOTE);

    // header: drag to move
    let hr = Rect::from_min_size(r.min, vec2(r.width() - hh, hh));
    let hresp = ui.interact(hr, Id::new(("nh", id)), Sense::click_and_drag());
    if hresp.dragged() {
        let d = hresp.drag_delta() / z;
        note.x += d.x;
        note.y += d.y;
        out.changed = true;
    }
    if hresp.clicked() || hresp.drag_started() {
        out.front = true;
    }
    p.text(
        pos2(r.min.x + 8.0 * z, r.min.y + hh / 2.0),
        Align2::LEFT_CENTER,
        "Note",
        FontId::proportional(11.0 * z),
        Color32::from_black_alpha(110),
    );

    // body: rich text editor
    let br = Rect::from_min_max(r.min + vec2(10.0 * z, hh), r.max - vec2(10.0 * z, 6.0 * z));
    let tid = Id::new(("nt", id));
    let bresp = ui.interact(br, Id::new(("nb", id)), Sense::click());
    if bresp.clicked() {
        ui.memory_mut(|m| m.request_focus(tid));
        out.front = true;
    }
    let before = note.text.clone();
    let size = FS * z;
    let runs = note.runs.clone();
    let bold_f = bold.clone();
    let mut layouter = |ui: &egui::Ui, text: &str, wrap: f32| {
        ui.fonts(|f| f.layout_job(build_job(text, &runs, size, wrap, &bold_f)))
    };
    let mut fmt_changed = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| {
        ui.set_clip_rect(br.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical()
            .id_salt(("ns", id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let te = egui::TextEdit::multiline(&mut note.text)
                    .id(tid)
                    .frame(false)
                    .desired_width(f32::INFINITY)
                    .desired_rows(1)
                    .layouter(&mut layouter)
                    .show(ui);
                if te.response.has_focus() {
                    if let Some(cr) = te.cursor_range {
                        let a = cr.primary.index;
                        let b = cr.secondary.index;
                        note.sel = (a.min(b), a.max(b));
                    }
                    for (k, which) in [(Key::B, 0u8), (Key::I, 1), (Key::U, 2)] {
                        if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, k)) {
                            toggle(note, which);
                            fmt_changed = true;
                        }
                    }
                }
                if te.response.clicked() || te.response.gained_focus() {
                    out.front = true;
                }
                if te.response.context_menu(|ui| format_menu(ui, note)) == Some(true) {
                    fmt_changed = true;
                }
            });
    });
    if fmt_changed {
        ui.memory_mut(|m| m.request_focus(tid));
        out.changed = true;
    }
    if note.text != before {
        reconcile(note, &before);
        out.changed = true;
    }

    // delete and resize last so they sit on top
    let xr = Rect::from_min_size(pos2(r.max.x - hh, r.min.y), vec2(hh, hh));
    let xresp = ui.interact(xr, Id::new(("nx", id)), Sense::click());
    p.text(
        xr.center(),
        Align2::CENTER_CENTER,
        "×",
        FontId::proportional(18.0 * z),
        if xresp.hovered() {
            INK
        } else {
            Color32::from_black_alpha(130)
        },
    );
    if xresp.clicked() {
        out.delete = true;
    }
    let rr = Rect::from_min_size(r.max - vec2(14.0 * z, 14.0 * z), vec2(14.0 * z, 14.0 * z));
    let rresp = ui.interact(rr, Id::new(("nr", id)), Sense::drag());
    if selected {
        p.rect_filled(rr, 2.0, ACC);
    }
    if rresp.hovered() || rresp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
    }
    if rresp.dragged() {
        let d = rresp.drag_delta() / z;
        note.w = (note.w + d.x).max(80.0);
        note.h = (note.h + d.y).max(60.0);
        out.changed = true;
    }
    out
}

// ---------- app ----------

struct App {
    data: Data,
    selected: Option<u64>,
    renaming: Option<usize>,
    closed: Vec<Tab>,
    zoom_req: Option<f32>,
    fit_req: bool,
    dirty: bool,
    path: PathBuf,
}

fn new_tab(n: usize) -> Tab {
    Tab {
        name: format!("Board {}", n),
        notes: Vec::new(),
        pan: [80.0, 80.0],
        zoom: 1.0,
    }
}

fn load_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let base: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let reg = fs::read(r"C:\Windows\Fonts\segoeui.ttf");
    let bld = fs::read(r"C:\Windows\Fonts\segoeuib.ttf");
    let mut prop = base.clone();
    let mut bold = base;
    if let Ok(b) = reg {
        fonts
            .font_data
            .insert("segoe".to_owned(), egui::FontData::from_owned(b).into());
        prop.insert(0, "segoe".to_owned());
    }
    if let Ok(b) = bld {
        fonts
            .font_data
            .insert("segoeb".to_owned(), egui::FontData::from_owned(b).into());
        bold.insert(0, "segoeb".to_owned());
    }
    fonts.families.insert(FontFamily::Proportional, prop);
    fonts.families.insert(FontFamily::Name("bold".into()), bold);
    ctx.set_fonts(fonts);
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        load_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        let dir = std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("BoardPad");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("board.json");
        let mut data: Data = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if data.tabs.is_empty() {
            data.tabs.push(new_tab(1));
        }
        if data.cur >= data.tabs.len() {
            data.cur = 0;
        }
        App {
            data,
            selected: None,
            renaming: None,
            closed: Vec::new(),
            zoom_req: None,
            fit_req: false,
            dirty: false,
            path,
        }
    }

    fn save(&mut self) {
        if let Ok(s) = serde_json::to_string(&self.data) {
            let tmp = self.path.with_extension("tmp");
            if fs::write(&tmp, s).is_ok() {
                let _ = fs::rename(&tmp, &self.path);
            }
        }
        self.dirty = false;
    }

    fn add_tab(&mut self) {
        let n = self.data.tabs.len() + 1;
        self.data.tabs.push(new_tab(n));
        self.data.cur = self.data.tabs.len() - 1;
        self.selected = None;
        self.dirty = true;
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let mut close: Option<usize> = None;
                let mut switch: Option<usize> = None;
                let mut add = false;
                let mut reopen = false;
                let room = (ui.available_width() - 330.0).max(100.0);
                egui::ScrollArea::horizontal()
                    .id_salt("tabs")
                    .max_width(room)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (i, t) in self.data.tabs.iter_mut().enumerate() {
                                if self.renaming == Some(i) {
                                    let r = ui.add(
                                        egui::TextEdit::singleline(&mut t.name)
                                            .desired_width(100.0),
                                    );
                                    if r.lost_focus() {
                                        self.renaming = None;
                                        self.dirty = true;
                                    } else {
                                        r.request_focus();
                                    }
                                } else {
                                    let resp = ui.selectable_label(i == self.data.cur, &t.name);
                                    if resp.clicked() {
                                        switch = Some(i);
                                    }
                                    if resp.double_clicked() {
                                        self.renaming = Some(i);
                                    }
                                    if resp.middle_clicked() {
                                        close = Some(i);
                                    }
                                    if ui.small_button("×").clicked() {
                                        close = Some(i);
                                    }
                                }
                            }
                            if ui.button("+").clicked() {
                                add = true;
                            }
                            if !self.closed.is_empty() && ui.button("Reopen").clicked() {
                                reopen = true;
                            }
                        });
                    });
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Fit").clicked() {
                        self.fit_req = true;
                    }
                    if ui.button("+").clicked() {
                        self.zoom_req = Some(1.25);
                    }
                    let z = self.data.tabs[self.data.cur].zoom;
                    ui.label(format!("{}%", (z * 100.0).round()));
                    if ui.button("−").clicked() {
                        self.zoom_req = Some(0.8);
                    }
                });
                if let Some(i) = switch {
                    self.data.cur = i;
                    self.selected = None;
                    self.dirty = true;
                }
                if add {
                    self.add_tab();
                }
                if reopen {
                    if let Some(t) = self.closed.pop() {
                        self.data.tabs.push(t);
                        self.data.cur = self.data.tabs.len() - 1;
                        self.dirty = true;
                    }
                }
                if let Some(i) = close {
                    let t = self.data.tabs.remove(i);
                    if !t.notes.is_empty() {
                        self.closed.push(t);
                        if self.closed.len() > 10 {
                            self.closed.remove(0);
                        }
                    }
                    if self.data.tabs.is_empty() {
                        self.data.tabs.push(new_tab(1));
                    }
                    if self.data.cur >= self.data.tabs.len() {
                        self.data.cur = self.data.tabs.len() - 1;
                    } else if i < self.data.cur {
                        self.data.cur -= 1;
                    }
                    self.renaming = None;
                    self.selected = None;
                    self.dirty = true;
                }
            });
        });
    }

    fn board(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();
        let bg = ui.interact(rect, Id::new("board"), Sense::click_and_drag());
        let bold = FontFamily::Name("bold".into());
        let zoom_req = self.zoom_req.take();
        let fit = std::mem::take(&mut self.fit_req);
        let mut selected = self.selected;
        let mut dirty = false;
        let hover = ctx.pointer_hover_pos().filter(|p| rect.contains(*p));
        // typing on an empty board starts a note
        let typed: String = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .concat()
        });
        let want_spawn = !typed.is_empty()
            && !ctx.wants_keyboard_input()
            && !typed.chars().all(|c| c.is_control());
        let cur = self.data.cur;
        let next_id = &mut self.data.next_id;
        let tab = &mut self.data.tabs[cur];

        if fit {
            tab.pan = [80.0, 80.0];
            tab.zoom = 1.0;
            dirty = true;
        }
        let mut over_note = false;
        if let Some(h) = hover {
            let z = tab.zoom;
            let org = rect.min + vec2(tab.pan[0], tab.pan[1]);
            over_note = tab.notes.iter().any(|n| {
                Rect::from_min_size(org + vec2(n.x, n.y) * z, vec2(n.w, n.h) * z).contains(h)
            });
        }
        // zoom around a screen point (cursor, or the centre for the +/- buttons)
        let mut factor = 1.0f32;
        let mut anchor = rect.center();
        if let Some(f) = zoom_req {
            factor = f;
        } else if hover.is_some() {
            let zd = ctx.input(|i| i.zoom_delta());
            if (zd - 1.0).abs() > 1e-4 {
                factor = zd;
                anchor = hover.unwrap_or(anchor);
            }
        }
        if (factor - 1.0).abs() > 1e-4 {
            let z0 = tab.zoom;
            let z1 = (z0 * factor).clamp(0.1, 4.0);
            let p = anchor - rect.min;
            tab.pan[0] = p.x - (p.x - tab.pan[0]) * z1 / z0;
            tab.pan[1] = p.y - (p.y - tab.pan[1]) * z1 / z0;
            tab.zoom = z1;
            dirty = true;
        }
        if hover.is_some() && !over_note {
            let sd = ctx.input(|i| i.smooth_scroll_delta);
            if sd != Vec2::ZERO {
                tab.pan[0] += sd.x;
                tab.pan[1] += sd.y;
                dirty = true;
            }
        }
        if bg.dragged_by(PointerButton::Primary) || bg.dragged_by(PointerButton::Middle) {
            let d = bg.drag_delta();
            tab.pan[0] += d.x;
            tab.pan[1] += d.y;
            dirty = true;
        }
        if bg.clicked() {
            selected = None;
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }

        // dotted background
        let painter = ui.painter_at(rect);
        let sp = 24.0 * tab.zoom;
        if sp >= 12.0 {
            let ox = tab.pan[0].rem_euclid(sp);
            let oy = tab.pan[1].rem_euclid(sp);
            let mut x = rect.min.x + ox;
            while x < rect.max.x {
                let mut y = rect.min.y + oy;
                while y < rect.max.y {
                    painter.circle_filled(pos2(x, y), 1.0, DOT);
                    y += sp;
                }
                x += sp;
            }
        }

        let view = View {
            origin: rect.min,
            pan: vec2(tab.pan[0], tab.pan[1]),
            z: tab.zoom,
        };

        let mut spawn_at: Option<Pos2> = None;
        if bg.double_clicked() {
            spawn_at = bg.interact_pointer_pos();
        } else if want_spawn {
            spawn_at = Some(
                hover
                    .map(|h| h + vec2(130.0, 100.0))
                    .unwrap_or(rect.center()),
            );
        }
        if let Some(sp_) = spawn_at {
            let z = view.z;
            let w = (sp_ - view.origin - view.pan) / z;
            let text = if bg.double_clicked() {
                String::new()
            } else {
                typed.clone()
            };
            let n = text.chars().count();
            *next_id += 1;
            let id = *next_id;
            tab.notes.push(Note {
                id,
                x: w.x - 130.0,
                y: w.y - 100.0,
                w: 260.0,
                h: 200.0,
                runs: if n > 0 {
                    vec![Run {
                        len: n,
                        st: Style::default(),
                    }]
                } else {
                    Vec::new()
                },
                text,
                sel: (n, n),
                typing: None,
            });
            selected = Some(id);
            let tid = Id::new(("nt", id));
            ctx.memory_mut(|m| m.request_focus(tid));
            let mut st = egui::text_edit::TextEditState::load(&ctx, tid).unwrap_or_default();
            st.cursor
                .set_char_range(Some(CCursorRange::one(CCursor::new(n))));
            st.store(&ctx, tid);
            dirty = true;
        }

        let mut delete: Option<u64> = None;
        let mut front: Option<u64> = None;
        for note in tab.notes.iter_mut() {
            let o = note_ui(ui, note, &view, selected == Some(note.id), &bold);
            if o.delete {
                delete = Some(note.id);
            }
            if o.front {
                front = Some(note.id);
            }
            if o.changed {
                dirty = true;
            }
        }
        if let Some(id) = delete {
            tab.notes.retain(|n| n.id != id);
            if selected == Some(id) {
                selected = None;
            }
            dirty = true;
        } else if let Some(id) = front {
            selected = Some(id);
            if let Some(i) = tab.notes.iter().position(|n| n.id == id) {
                if i + 1 != tab.notes.len() {
                    let n = tab.notes.remove(i);
                    tab.notes.push(n);
                }
            }
        }
        if tab.notes.is_empty() {
            let h = "Just start typing to add a note";
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                h,
                FontId::proportional(15.0),
                Color32::from_black_alpha(100),
            );
        }
        self.selected = selected;
        if dirty {
            self.dirty = true;
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.top_bar(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(BG))
            .show(ctx, |ui| self.board(ui));
        // save right away, except mid-drag (saved on release)
        if self.dirty && !ctx.input(|i| i.pointer.any_down()) {
            self.save();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save();
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("Board Pad"),
        ..Default::default()
    };
    eframe::run_native(
        "Board Pad",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{
    self, pos2,
    text::{CCursor, CCursorRange, LayoutJob, TextFormat},
    vec2, Align, Align2, Color32, FontFamily, FontId, Id, Key, Modifiers, PointerButton, Pos2,
    Rect, Sense, Stroke, Vec2,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

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
struct Img {
    id: u64,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

// a pin is free (x,y = world position) or attached to a note/image (x,y = offset from its top-left)
#[derive(Clone, Serialize, Deserialize)]
struct Pin {
    id: u64,
    x: f32,
    y: f32,
    at: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Tab {
    name: String,
    notes: Vec<Note>,
    pan: [f32; 2],
    zoom: f32,
    #[serde(default)]
    images: Vec<Img>,
    #[serde(default)]
    pins: Vec<Pin>,
    #[serde(default)]
    strings: Vec<(u64, u64)>,
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

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Select,
    Pin,
    Hand,
}

// what one item reports back; the board applies moves/selection for the whole group
#[derive(Default)]
struct ItemOut {
    delete: bool,
    clicked: bool,
    started: bool,
    stopped: bool,
    changed: bool,
    tie: bool,
    cut: bool,
    drag: Vec2,
}

// lazy texture cache: images live as PNG files in %APPDATA%/BoardPad/img
struct Tex {
    map: HashMap<u64, egui::TextureHandle>,
    bad: HashSet<u64>,
    dir: PathBuf,
    loaded: bool,
}

impl Tex {
    // loads at most one texture per frame so big boards never stall
    fn get(&mut self, ctx: &egui::Context, id: u64) -> Option<egui::TextureId> {
        if let Some(t) = self.map.get(&id) {
            return Some(t.id());
        }
        if self.bad.contains(&id) {
            return None;
        }
        if self.loaded {
            ctx.request_repaint();
            return None;
        }
        self.loaded = true;
        match image::open(self.dir.join(format!("{}.png", id))) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let ci = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                let h = ctx.load_texture(format!("i{}", id), ci, egui::TextureOptions::LINEAR);
                let tid = h.id();
                self.map.insert(id, h);
                ctx.request_repaint();
                Some(tid)
            }
            Err(_) => {
                self.bad.insert(id);
                None
            }
        }
    }
}

fn host_rects(t: &Tab) -> HashMap<u64, Rect> {
    t.images
        .iter()
        .map(|m| (m.id, Rect::from_min_size(pos2(m.x, m.y), vec2(m.w, m.h))))
        .chain(
            t.notes
                .iter()
                .map(|n| (n.id, Rect::from_min_size(pos2(n.x, n.y), vec2(n.w, n.h)))),
        )
        .collect()
}

fn pin_world(p: &Pin, h: &HashMap<u64, Rect>) -> Pos2 {
    match p.at.and_then(|i| h.get(&i)) {
        Some(r) => r.min + vec2(p.x, p.y),
        None => pos2(p.x, p.y),
    }
}

// attach each listed pin to the top-most note/image under it, or free it
fn rehost(t: &mut Tab, ids: &HashSet<u64>) {
    let h = host_rects(t);
    let order: Vec<u64> = t
        .images
        .iter()
        .map(|m| m.id)
        .chain(t.notes.iter().map(|n| n.id))
        .collect();
    for p in t.pins.iter_mut().filter(|p| ids.contains(&p.id)) {
        let w = pin_world(p, &h);
        let host = order.iter().rev().copied().find(|i| h[i].contains(w));
        match host {
            Some(i) => {
                p.at = Some(i);
                let o = w - h[&i].min;
                p.x = o.x;
                p.y = o.y;
            }
            None => {
                p.at = None;
                p.x = w.x;
                p.y = w.y;
            }
        }
    }
}

fn delete_items(t: &mut Tab, ids: &HashSet<u64>) {
    let h = host_rects(t);
    for p in t.pins.iter_mut() {
        if let Some(a) = p.at {
            if ids.contains(&a) && !ids.contains(&p.id) {
                let w = pin_world(p, &h);
                p.at = None;
                p.x = w.x;
                p.y = w.y;
            }
        }
    }
    t.notes.retain(|n| !ids.contains(&n.id));
    t.images.retain(|m| !ids.contains(&m.id));
    t.pins.retain(|p| !ids.contains(&p.id));
    t.strings
        .retain(|(a, b)| !ids.contains(a) && !ids.contains(b));
}

fn add_image(
    dir: &Path,
    next_id: &mut u64,
    t: &mut Tab,
    img: image::DynamicImage,
    at: Vec2,
) -> Option<u64> {
    let img = if img.width().max(img.height()) > 1600 {
        img.resize(1600, 1600, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    *next_id += 1;
    let id = *next_id;
    let _ = fs::create_dir_all(dir);
    img.save_with_format(dir.join(format!("{}.png", id)), image::ImageFormat::Png)
        .ok()?;
    let (pw, ph) = (img.width() as f32, img.height() as f32);
    let w = pw.min(360.0);
    let h = w * ph / pw;
    t.images.push(Img {
        id,
        x: at.x - w / 2.0,
        y: at.y - h / 2.0,
        w,
        h,
    });
    Some(id)
}

fn note_ui(
    ui: &mut egui::Ui,
    note: &mut Note,
    v: &View,
    selected: bool,
    live: bool,
    bold: &FontFamily,
) -> ItemOut {
    let z = v.z;
    let id = note.id;
    let mut out = ItemOut::default();
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

    // header: drag to move (the board moves the whole selection)
    if live {
        let hr = Rect::from_min_size(r.min, vec2(r.width() - hh, hh));
        let hresp = ui.interact(hr, Id::new(("nh", id)), Sense::click_and_drag());
        if hresp.dragged() {
            out.drag = hresp.drag_delta() / z;
        }
        out.started |= hresp.drag_started();
        out.stopped |= hresp.drag_stopped();
        out.clicked |= hresp.clicked();
    }
    p.text(
        pos2(r.min.x + 8.0 * z, r.min.y + hh / 2.0),
        Align2::LEFT_CENTER,
        "Note",
        FontId::proportional(11.0 * z),
        Color32::from_black_alpha(110),
    );

    let br = Rect::from_min_max(r.min + vec2(10.0 * z, hh), r.max - vec2(10.0 * z, 6.0 * z));
    if live {
        // body: rich text editor
        let tid = Id::new(("nt", id));
        let bresp = ui.interact(br, Id::new(("nb", id)), Sense::click());
        if bresp.clicked() {
            ui.memory_mut(|m| m.request_focus(tid));
            out.clicked = true;
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
                        out.clicked = true;
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
    } else {
        // read-only paint while the Pin/Hand tool is active
        let job = build_job(&note.text, &note.runs, FS * z, br.width(), bold);
        let g = ui.fonts(|f| f.layout_job(job));
        ui.painter_at(br).galley(br.min, g, INK);
    }

    // delete and resize last so they sit on top
    let xr = Rect::from_min_size(pos2(r.max.x - hh, r.min.y), vec2(hh, hh));
    let mut xhover = false;
    if live {
        let xresp = ui.interact(xr, Id::new(("nx", id)), Sense::click());
        xhover = xresp.hovered();
        if xresp.clicked() {
            out.delete = true;
        }
    }
    p.text(
        xr.center(),
        Align2::CENTER_CENTER,
        "×",
        FontId::proportional(18.0 * z),
        if xhover {
            INK
        } else {
            Color32::from_black_alpha(130)
        },
    );
    if live {
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
    }
    out
}

fn img_ui(
    ui: &mut egui::Ui,
    im: &mut Img,
    v: &View,
    selected: bool,
    live: bool,
    tex: &mut Tex,
) -> ItemOut {
    let z = v.z;
    let mut out = ItemOut::default();
    let r = Rect::from_min_size(
        v.origin + v.pan + vec2(im.x, im.y) * z,
        vec2(im.w, im.h) * z,
    );
    if !r.intersects(ui.clip_rect()) {
        return out;
    }
    let p = ui.painter().clone();
    if selected {
        p.rect_filled(r.expand(2.0), 0.0, ACC);
    }
    match tex.get(ui.ctx(), im.id) {
        Some(t) => p.image(
            t,
            r,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        ),
        None => p.rect_filled(r, 0.0, Color32::from_gray(225)),
    }
    if live {
        let resp = ui.interact(r, Id::new(("im", im.id)), Sense::click_and_drag());
        if resp.dragged() {
            out.drag = resp.drag_delta() / z;
        }
        out.started |= resp.drag_started();
        out.stopped |= resp.drag_stopped();
        out.clicked |= resp.clicked();
        resp.context_menu(|ui| {
            if ui.button("Delete").clicked() {
                out.delete = true;
                ui.close_menu();
            }
        });
        // resize corner keeps the aspect ratio
        let rr = Rect::from_min_size(r.max - vec2(14.0 * z, 14.0 * z), vec2(14.0 * z, 14.0 * z));
        let rresp = ui.interact(rr, Id::new(("ir", im.id)), Sense::drag());
        if selected {
            p.rect_filled(rr, 2.0, ACC);
        }
        if rresp.hovered() || rresp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
        }
        if rresp.dragged() {
            let ratio = im.h / im.w;
            im.w = (im.w + rresp.drag_delta().x / z).max(40.0);
            im.h = im.w * ratio;
            out.changed = true;
        }
    }
    out
}

fn pin_ui(
    ui: &mut egui::Ui,
    pin: &Pin,
    c: Pos2,
    z: f32,
    selected: bool,
    live: bool,
    can_tie: bool,
) -> ItemOut {
    let mut out = ItemOut::default();
    if !ui.clip_rect().expand(24.0).contains(c) {
        return out;
    }
    let rad = (7.0 * z).clamp(4.0, 14.0);
    let p = ui.painter().clone();
    p.circle_filled(c + vec2(1.0, 2.0), rad, Color32::from_black_alpha(60));
    if selected {
        p.circle_filled(c, rad + 3.0, ACC);
    }
    p.circle_filled(c, rad, Color32::from_rgb(0xd1, 0x34, 0x38));
    p.circle_filled(
        c - vec2(rad * 0.3, rad * 0.3),
        rad * 0.3,
        Color32::from_white_alpha(150),
    );
    if live {
        let hr = Rect::from_center_size(c, Vec2::splat(rad * 2.0 + 8.0));
        let resp = ui.interact(hr, Id::new(("pn", pin.id)), Sense::click_and_drag());
        if resp.dragged() {
            out.drag = resp.drag_delta() / z;
        }
        out.started |= resp.drag_started();
        out.stopped |= resp.drag_stopped();
        out.clicked |= resp.clicked();
        resp.context_menu(|ui| {
            if ui
                .add_enabled(can_tie, egui::Button::new("Tie with string"))
                .clicked()
            {
                out.tie = true;
                ui.close_menu();
            }
            if ui.button("Cut strings").clicked() {
                out.cut = true;
                ui.close_menu();
            }
            if ui.button("Delete").clicked() {
                out.delete = true;
                ui.close_menu();
            }
        });
    }
    out
}

// ---------- app ----------

struct App {
    data: Data,
    selected: HashSet<u64>,
    tool: Tool,
    marquee: Option<Pos2>,
    tex: Tex,
    img_dir: PathBuf,
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
        images: Vec::new(),
        pins: Vec::new(),
        strings: Vec::new(),
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
        let img_dir = dir.join("img");
        App {
            data,
            selected: HashSet::new(),
            tool: Tool::Select,
            marquee: None,
            tex: Tex {
                map: HashMap::new(),
                bad: HashSet::new(),
                dir: img_dir.clone(),
                loaded: false,
            },
            img_dir,
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
        self.selected.clear();
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
                    self.selected.clear();
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
                    if !(t.notes.is_empty() && t.images.is_empty() && t.pins.is_empty()) {
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
                    self.selected.clear();
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
        let tool = self.tool;
        let kb_free = !ctx.wants_keyboard_input();
        let space = kb_free && ctx.input(|i| i.key_down(Key::Space));
        let hand = tool == Tool::Hand || space;
        let live = tool == Tool::Select && !space;
        let mut sel = std::mem::take(&mut self.selected);
        let mut dirty = false;
        self.tex.loaded = false;
        let hover = ctx.pointer_hover_pos().filter(|p| rect.contains(*p));

        let (typed, pasted, paste_key, multi, del_key, all_key, esc) = ctx.input(|i| {
            let mut t = String::new();
            let mut pa = String::new();
            for e in &i.events {
                match e {
                    egui::Event::Text(s) => t.push_str(s),
                    egui::Event::Paste(s) => pa.push_str(s),
                    _ => {}
                }
            }
            (
                t,
                pa,
                i.modifiers.command && i.key_pressed(Key::V),
                i.modifiers.command || i.modifiers.shift,
                i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
                i.modifiers.command && i.key_pressed(Key::A),
                i.key_pressed(Key::Escape),
            )
        });
        if esc {
            sel.clear();
            self.tool = Tool::Select;
        }

        // images from the clipboard or dropped files
        let mut incoming: Vec<image::DynamicImage> = Vec::new();
        if paste_key && kb_free {
            if let Ok(mut cb) = arboard::Clipboard::new() {
                if let Ok(d) = cb.get_image() {
                    if let Some(b) = image::RgbaImage::from_raw(
                        d.width as u32,
                        d.height as u32,
                        d.bytes.into_owned(),
                    ) {
                        incoming.push(image::DynamicImage::ImageRgba8(b));
                    }
                }
            }
        }
        for f in ctx.input(|i| i.raw.dropped_files.clone()) {
            let img = if let Some(p) = &f.path {
                image::open(p).ok()
            } else if let Some(b) = &f.bytes {
                image::load_from_memory(b).ok()
            } else {
                None
            };
            if let Some(i) = img {
                incoming.push(i);
            }
        }

        // pasted text or typing on the board starts a note
        let spawn_text: Option<String> = if kb_free && incoming.is_empty() {
            if !pasted.trim().is_empty() {
                Some(pasted)
            } else if tool == Tool::Select
                && !typed.trim().is_empty()
                && !typed.chars().all(|c| c.is_control())
            {
                Some(typed)
            } else {
                None
            }
        } else {
            None
        };

        let cur = self.data.cur;
        let next_id = &mut self.data.next_id;
        let tab = &mut self.data.tabs[cur];

        if all_key && kb_free {
            sel = tab
                .notes
                .iter()
                .map(|n| n.id)
                .chain(tab.images.iter().map(|m| m.id))
                .chain(tab.pins.iter().map(|p| p.id))
                .collect();
        }
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
        // pan: middle drag always, left drag with the Hand tool or Space held
        if bg.dragged_by(PointerButton::Middle) || (hand && bg.dragged_by(PointerButton::Primary)) {
            let d = bg.drag_delta();
            tab.pan[0] += d.x;
            tab.pan[1] += d.y;
            dirty = true;
        }
        if hand && hover.is_some() {
            ctx.set_cursor_icon(if bg.dragged() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            });
        }

        let view = View {
            origin: rect.min,
            pan: vec2(tab.pan[0], tab.pan[1]),
            z: tab.zoom,
        };
        let to_world = |p: Pos2| (p - view.origin - view.pan) / view.z;
        let to_screen = |w: Pos2| view.origin + view.pan + w.to_vec2() * view.z;

        if bg.clicked() {
            if tool == Tool::Pin {
                if let Some(sp) = bg.interact_pointer_pos() {
                    let w = to_world(sp);
                    *next_id += 1;
                    let id = *next_id;
                    tab.pins.push(Pin {
                        id,
                        x: w.x,
                        y: w.y,
                        at: None,
                    });
                    let ids = HashSet::from([id]);
                    rehost(tab, &ids);
                    sel = ids;
                    dirty = true;
                }
            } else if tool == Tool::Select && !multi {
                sel.clear();
                ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
            }
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

        // new notes and images
        let mut spawn_at: Option<Pos2> = None;
        let dbl = tool == Tool::Select && bg.double_clicked();
        if dbl {
            spawn_at = bg.interact_pointer_pos();
        } else if spawn_text.is_some() {
            spawn_at = Some(
                hover
                    .map(|h| h + vec2(130.0, 100.0))
                    .unwrap_or(rect.center()),
            );
        }
        if let Some(sp_) = spawn_at {
            let w = to_world(sp_);
            let text = spawn_text.clone().unwrap_or_default();
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
            sel = HashSet::from([id]);
            let tid = Id::new(("nt", id));
            ctx.memory_mut(|m| m.request_focus(tid));
            let mut st = egui::text_edit::TextEditState::load(&ctx, tid).unwrap_or_default();
            st.cursor
                .set_char_range(Some(CCursorRange::one(CCursor::new(n))));
            st.store(&ctx, tid);
            dirty = true;
        }
        if !incoming.is_empty() {
            let at = to_world(hover.unwrap_or(rect.center())).to_vec2();
            sel.clear();
            for (k, img) in incoming.into_iter().enumerate() {
                let off = vec2(24.0, 24.0) * k as f32;
                if let Some(id) = add_image(&self.img_dir, next_id, tab, img, at + off) {
                    sel.insert(id);
                    dirty = true;
                }
            }
        }

        // items: images, notes, strings, pins (bottom to top)
        let hosts = host_rects(tab);
        let pw: HashMap<u64, Pos2> = tab
            .pins
            .iter()
            .map(|p| (p.id, pin_world(p, &hosts)))
            .collect();
        let mut acts: Vec<(u64, ItemOut)> = Vec::new();
        for m in tab.images.iter_mut() {
            let o = img_ui(ui, m, &view, sel.contains(&m.id), live, &mut self.tex);
            acts.push((m.id, o));
        }
        for n in tab.notes.iter_mut() {
            let o = note_ui(ui, n, &view, sel.contains(&n.id), live, &bold);
            acts.push((n.id, o));
        }
        let string_col = Color32::from_rgb(0xc0, 0x28, 0x2d);
        for (a, b) in &tab.strings {
            if let (Some(&pa), Some(&pb)) = (pw.get(a), pw.get(b)) {
                painter.line_segment(
                    [to_screen(pa), to_screen(pb)],
                    Stroke::new((2.0 * view.z).max(1.0), string_col),
                );
            }
        }
        let sel_pins: Vec<u64> = tab
            .pins
            .iter()
            .filter(|p| sel.contains(&p.id))
            .map(|p| p.id)
            .collect();
        let tie_ok = sel_pins.len() == 2
            && !tab.strings.iter().any(|&(a, b)| {
                (a == sel_pins[0] && b == sel_pins[1]) || (a == sel_pins[1] && b == sel_pins[0])
            });
        for p in tab.pins.iter() {
            let c = to_screen(pw[&p.id]);
            let can_tie = tie_ok && sel_pins.contains(&p.id);
            let o = pin_ui(ui, p, c, view.z, sel.contains(&p.id), live, can_tie);
            acts.push((p.id, o));
        }

        // selection from clicks and drag starts
        for (id, o) in &acts {
            if o.started || o.clicked {
                if multi {
                    if o.clicked && !sel.remove(id) {
                        sel.insert(*id);
                    } else if o.started {
                        sel.insert(*id);
                    }
                } else if !sel.contains(id) || (o.clicked && sel.len() > 1) {
                    sel.clear();
                    sel.insert(*id);
                }
            }
        }
        // bring the clicked note/image to the front
        if let Some((id, _)) = acts.iter().find(|(_, o)| o.clicked || o.started) {
            if let Some(i) = tab.notes.iter().position(|n| n.id == *id) {
                if i + 1 != tab.notes.len() {
                    let n = tab.notes.remove(i);
                    tab.notes.push(n);
                }
            } else if let Some(i) = tab.images.iter().position(|m| m.id == *id) {
                if i + 1 != tab.images.len() {
                    let m = tab.images.remove(i);
                    tab.images.push(m);
                }
            }
        }
        // move the whole selection
        let mv = acts.iter().fold(Vec2::ZERO, |a, (_, o)| a + o.drag);
        if mv != Vec2::ZERO {
            for n in tab.notes.iter_mut().filter(|n| sel.contains(&n.id)) {
                n.x += mv.x;
                n.y += mv.y;
            }
            for m in tab.images.iter_mut().filter(|m| sel.contains(&m.id)) {
                m.x += mv.x;
                m.y += mv.y;
            }
            // pins riding on a selected host move with it already
            for p in tab.pins.iter_mut().filter(|p| sel.contains(&p.id)) {
                if p.at.map_or(false, |h| sel.contains(&h)) {
                    continue;
                }
                p.x += mv.x;
                p.y += mv.y;
            }
            dirty = true;
        }
        if acts.iter().any(|(_, o)| o.stopped) {
            rehost(tab, &sel);
            dirty = true;
        }
        if acts.iter().any(|(_, o)| o.changed) {
            dirty = true;
        }
        // tie / cut strings
        for (id, o) in &acts {
            if o.tie && tie_ok {
                tab.strings.push((sel_pins[0], sel_pins[1]));
                dirty = true;
            }
            if o.cut {
                tab.strings.retain(|&(a, b)| a != *id && b != *id);
                dirty = true;
            }
        }
        // delete
        let mut del: HashSet<u64> = acts
            .iter()
            .filter(|(_, o)| o.delete)
            .map(|(id, _)| *id)
            .collect();
        if kb_free && del_key && !sel.is_empty() {
            del = sel.clone();
        }
        if !del.is_empty() {
            delete_items(tab, &del);
            sel.retain(|id| !del.contains(id));
            dirty = true;
        }

        // marquee selection
        if live && bg.drag_started_by(PointerButton::Primary) {
            self.marquee = bg.interact_pointer_pos();
        }
        if let (Some(s), Some(now)) = (self.marquee, ctx.pointer_latest_pos()) {
            let mr = Rect::from_two_pos(s, now);
            if bg.drag_stopped() {
                let rs = |x: f32, y: f32, w: f32, h: f32| {
                    Rect::from_min_size(to_screen(pos2(x, y)), vec2(w, h) * view.z)
                };
                let mut hit: HashSet<u64> = HashSet::new();
                for n in &tab.notes {
                    if rs(n.x, n.y, n.w, n.h).intersects(mr) {
                        hit.insert(n.id);
                    }
                }
                for m in &tab.images {
                    if rs(m.x, m.y, m.w, m.h).intersects(mr) {
                        hit.insert(m.id);
                    }
                }
                for p in &tab.pins {
                    if let Some(&w) = pw.get(&p.id) {
                        if Rect::from_center_size(to_screen(w), Vec2::splat(16.0)).intersects(mr) {
                            hit.insert(p.id);
                        }
                    }
                }
                if multi {
                    sel.extend(hit);
                } else {
                    sel = hit;
                }
                self.marquee = None;
            } else if bg.dragged() {
                painter.rect_filled(mr, 0.0, ACC.gamma_multiply(0.12));
                painter.add(egui::Shape::closed_line(
                    vec![
                        mr.left_top(),
                        mr.right_top(),
                        mr.right_bottom(),
                        mr.left_bottom(),
                    ],
                    Stroke::new(1.0, ACC),
                ));
            } else {
                self.marquee = None;
            }
        }

        if tab.notes.is_empty() && tab.images.is_empty() && tab.pins.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Just start typing to add a note, or paste / drop an image",
                FontId::proportional(15.0),
                Color32::from_black_alpha(100),
            );
        }
        self.selected = sel;
        if dirty {
            self.dirty = true;
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.top_bar(ctx);
        egui::TopBottomPanel::top("tools").show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (t, l) in [
                    (Tool::Select, "Select"),
                    (Tool::Pin, "Pin"),
                    (Tool::Hand, "Hand"),
                ] {
                    ui.selectable_value(&mut self.tool, t, l);
                }
                ui.separator();
                ui.weak(
                    "Ctrl+click or drag a box to multi-select · right-click a pin to tie strings",
                );
            });
        });
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

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

// one pen/calligraphy stroke; pts are world coords, dense enough for the eraser
#[derive(Clone, Serialize, Deserialize)]
struct Ink {
    pts: Vec<[f32; 2]>,
    color: [u8; 3],
    size: f32,
    alpha: u8,
    cal: bool,
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
    #[serde(default)]
    strokes: Vec<Ink>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Cfg {
    dark: bool,
    mini: bool,
    auto: bool,
    skip: String,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg {
            dark: false,
            mini: true,
            auto: true,
            skip: String::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Data {
    tabs: Vec<Tab>,
    cur: usize,
    next_id: u64,
    #[serde(default)]
    cfg: Cfg,
}

// GitHub "owner/name" that publishes the releases; update check is skipped until set
const REPO: &str = "owner/repo";

fn ver_nums(v: &str) -> (Vec<u32>, bool) {
    let v = v.trim_start_matches('v');
    let core = v.split('-').next().unwrap_or("");
    (
        core.split('.').filter_map(|x| x.parse().ok()).collect(),
        v.contains('-'),
    )
}

fn is_newer(tag: &str, cur: &str) -> bool {
    let (a, ap) = ver_nums(tag);
    let (b, bp) = ver_nums(cur);
    a > b || (a == b && bp && !ap)
}

// uses curl.exe (ships with Windows 10+), so no HTTP/TLS crate in the binary
fn latest_tag() -> Option<String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);
    let mut c = std::process::Command::new("curl.exe");
    c.args(["-s", "-m", "6", "-H", "User-Agent: board-pad", url.as_str()]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let out = c.output().ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v["tag_name"].as_str().map(|t| t.to_string())
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
                Stroke::new(1.0_f32, c)
            } else {
                Stroke::NONE
            },
            strikethrough: if st.s {
                Stroke::new(1.0_f32, c)
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
    Draw,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Pen,
    Cal,
    Eraser,
}

struct Brush {
    kind: Kind,
    color: [u8; 3],
    size: f32,
    opacity: u8, // percent
}

fn eraser_r(b: &Brush) -> f32 {
    b.size.max(12.0) / 2.0
}

// append a point, filling the gap so strokes stay dense
fn add_pts(pts: &mut Vec<[f32; 2]>, p: Vec2, step: f32) {
    if let Some(l) = pts.last().copied() {
        let d = vec2(p.x - l[0], p.y - l[1]);
        let len = d.length();
        if len < step {
            return;
        }
        let n = (len / step).ceil() as usize;
        for k in 1..=n {
            let t = k as f32 / n as f32;
            pts.push([l[0] + d.x * t, l[1] + d.y * t]);
        }
    } else {
        pts.push([p.x, p.y]);
    }
}

fn path_steps(a: Vec2, b: Vec2, step: f32) -> Vec<Vec2> {
    let n = ((b - a).length() / step).ceil().max(1.0) as usize;
    (1..=n)
        .map(|k| a + (b - a) * (k as f32 / n as f32))
        .collect()
}

// remove the part of each stroke under the eraser, splitting strokes in two
fn erase(v: &mut Vec<Ink>, c: Vec2, r: f32) -> bool {
    let r2 = r * r;
    let hit = |p: &[f32; 2]| (p[0] - c.x).powi(2) + (p[1] - c.y).powi(2) <= r2;
    if !v.iter().any(|k| k.pts.iter().any(hit)) {
        return false;
    }
    let mut out: Vec<Ink> = Vec::with_capacity(v.len() + 1);
    for k in v.drain(..) {
        if !k.pts.iter().any(hit) {
            out.push(k);
            continue;
        }
        let mut run: Vec<[f32; 2]> = Vec::new();
        for p in &k.pts {
            if hit(p) {
                if !run.is_empty() {
                    out.push(Ink {
                        pts: std::mem::take(&mut run),
                        color: k.color,
                        size: k.size,
                        alpha: k.alpha,
                        cal: k.cal,
                    });
                }
            } else {
                run.push(*p);
            }
        }
        if !run.is_empty() {
            out.push(Ink {
                pts: run,
                color: k.color,
                size: k.size,
                alpha: k.alpha,
                cal: k.cal,
            });
        }
    }
    *v = out;
    true
}

fn paint_ink(p: &egui::Painter, ink: &Ink, org: Vec2, z: f32, clip: Rect) {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let pts: Vec<Pos2> = ink
        .pts
        .iter()
        .map(|q| {
            let s = pos2(q[0] * z + org.x, q[1] * z + org.y);
            x0 = x0.min(s.x);
            y0 = y0.min(s.y);
            x1 = x1.max(s.x);
            y1 = y1.max(s.y);
            s
        })
        .collect();
    let pad = ink.size * z;
    if pts.is_empty()
        || !Rect::from_min_max(pos2(x0 - pad, y0 - pad), pos2(x1 + pad, y1 + pad)).intersects(clip)
    {
        return;
    }
    let col = Color32::from_rgba_unmultiplied(ink.color[0], ink.color[1], ink.color[2], ink.alpha);
    let w = (ink.size * z).max(1.0);
    if ink.cal {
        // flat nib held at 45 degrees
        let a = std::f32::consts::FRAC_PI_4;
        let n = vec2(a.cos(), -a.sin()) * w * 0.5;
        if pts.len() == 1 {
            p.line_segment([pts[0] - n, pts[0] + n], Stroke::new(1.5_f32, col));
            return;
        }
        let mut m = egui::Mesh::default();
        for s in pts.windows(2) {
            let i = m.vertices.len() as u32;
            m.colored_vertex(s[0] - n, col);
            m.colored_vertex(s[0] + n, col);
            m.colored_vertex(s[1] + n, col);
            m.colored_vertex(s[1] - n, col);
            m.add_triangle(i, i + 1, i + 2);
            m.add_triangle(i, i + 2, i + 3);
        }
        p.add(egui::Shape::mesh(m));
    } else if pts.len() == 1 {
        p.circle_filled(pts[0], w / 2.0, col);
    } else {
        p.add(egui::Shape::line(pts, Stroke::new(w, col)));
    }
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
                            let a = cr.primary.ccursor.index;
                            let b = cr.secondary.ccursor.index;
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
                    te.response.context_menu(|ui| {
                        if format_menu(ui, note) {
                            fmt_changed = true;
                        }
                    });
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
    };
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
    search: Option<String>,
    goto: Option<u64>,
    settings_open: bool,
    rx: Option<std::sync::mpsc::Receiver<Option<String>>>,
    upd: Option<String>,
    upd_msg: String,
    brush: Brush,
    hist: Vec<Vec<Ink>>,
    redo: Vec<Vec<Ink>>,
    hist_key: (usize, usize),
    cur_ink: Option<Ink>,
    last_e: Option<Vec2>,
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
        strokes: Vec::new(),
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
        cc.egui_ctx.set_visuals(if data.cfg.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        let mut app = App {
            data,
            selected: HashSet::new(),
            tool: Tool::Select,
            marquee: None,
            search: None,
            goto: None,
            settings_open: false,
            rx: None,
            upd: None,
            upd_msg: String::new(),
            brush: Brush {
                kind: Kind::Pen,
                color: PALETTE[0],
                size: 4.0,
                opacity: 100,
            },
            hist: Vec::new(),
            redo: Vec::new(),
            hist_key: (0, 0),
            cur_ink: None,
            last_e: None,
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
        };
        if app.data.cfg.auto {
            app.start_check(&cc.egui_ctx);
        }
        app
    }

    fn start_check(&mut self, ctx: &egui::Context) {
        if REPO.starts_with("owner/") {
            self.upd_msg = "Set REPO in main.rs first".into();
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(latest_tag());
            c.request_repaint();
        });
        self.rx = Some(rx);
        self.upd_msg = "Checking…".into();
    }

    fn settings_ui(&mut self, ctx: &egui::Context) {
        let res = self.rx.as_ref().and_then(|r| r.try_recv().ok());
        if let Some(res) = res {
            self.rx = None;
            match res {
                Some(tag) if is_newer(&tag, env!("CARGO_PKG_VERSION")) => {
                    self.upd_msg = format!("{} is available", tag);
                    self.upd = Some(tag);
                }
                Some(_) => self.upd_msg = "You are up to date".into(),
                None => self.upd_msg = "Could not reach GitHub".into(),
            }
        }
        if let Some(tag) = self.upd.clone() {
            if tag != self.data.cfg.skip {
                egui::Window::new("update")
                    .title_bar(false)
                    .resizable(false)
                    .anchor(Align2::LEFT_BOTTOM, vec2(12.0, -12.0))
                    .show(ctx, |ui| {
                        ui.label(format!("Board Pad {} is out", tag));
                        ui.horizontal(|ui| {
                            if ui.button("Download").clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(format!(
                                    "https://github.com/{}/releases/latest",
                                    REPO
                                )));
                                self.upd = None;
                            }
                            if ui.button("Later").clicked() {
                                self.upd = None;
                            }
                            if ui.button("Skip this version").clicked() {
                                self.data.cfg.skip = tag.clone();
                                self.upd = None;
                                self.dirty = true;
                            }
                        });
                    });
            }
        }
        let mut open = self.settings_open;
        let mut check = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                if ui.checkbox(&mut self.data.cfg.dark, "Dark theme").changed() {
                    ctx.set_visuals(if self.data.cfg.dark {
                        egui::Visuals::dark()
                    } else {
                        egui::Visuals::light()
                    });
                    self.dirty = true;
                }
                if ui
                    .checkbox(&mut self.data.cfg.mini, "Show minimap")
                    .changed()
                {
                    self.dirty = true;
                }
                if ui
                    .checkbox(&mut self.data.cfg.auto, "Check for updates on start")
                    .changed()
                {
                    self.dirty = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("Check now").clicked() {
                        check = true;
                    }
                    ui.label(&self.upd_msg);
                });
                ui.separator();
                ui.weak(format!("Board Pad {}", env!("CARGO_PKG_VERSION")));
            });
        self.settings_open = open;
        if check {
            self.data.cfg.skip.clear();
            self.start_check(ctx);
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
        let dark = self.data.cfg.dark;
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let mut close: Option<usize> = None;
                let mut switch: Option<usize> = None;
                let mut add = false;
                let mut reopen = false;
                let room = (ui.available_width() - 420.0).max(100.0);
                let n = self.data.tabs.len();
                let tw = ((room - 8.0) / n as f32).clamp(40.0, 150.0);
                let strip = Rect::from_min_size(ui.cursor().min, vec2(room, 26.0));
                let mut reorder: Option<(usize, usize)> = None;
                egui::ScrollArea::horizontal()
                    .id_salt("tabs")
                    .max_width(room)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        if ctx.pointer_hover_pos().map_or(false, |p| strip.contains(p)) {
                            let d = ctx.input(|i| i.smooth_scroll_delta);
                            if d != Vec2::ZERO {
                                ui.scroll_with_delta(vec2(d.y + d.x, 0.0));
                            }
                        }
                        let mut rects: Vec<Rect> = Vec::with_capacity(n);
                        let mut drag_from: Option<usize> = None;
                        let mut dropped = false;
                        for i in 0..n {
                            if self.renaming == Some(i) {
                                let r = ui.add_sized(
                                    [tw.max(100.0), 24.0],
                                    egui::TextEdit::singleline(&mut self.data.tabs[i].name),
                                );
                                rects.push(r.rect);
                                if r.lost_focus() {
                                    self.renaming = None;
                                    self.dirty = true;
                                } else {
                                    r.request_focus();
                                }
                                continue;
                            }
                            let (rect, resp) =
                                ui.allocate_exact_size(vec2(tw, 24.0), Sense::click_and_drag());
                            rects.push(rect);
                            let cur = i == self.data.cur;
                            let p = ui.painter().clone();
                            let fill = match (dark, cur, resp.hovered()) {
                                (false, true, _) => Color32::WHITE,
                                (false, false, true) => Color32::from_gray(232),
                                (false, false, false) => Color32::from_gray(222),
                                (true, true, _) => Color32::from_gray(64),
                                (true, false, true) => Color32::from_gray(50),
                                (true, false, false) => Color32::from_gray(38),
                            };
                            p.rect_filled(rect, 4.0, fill);
                            if cur {
                                p.rect_filled(
                                    Rect::from_min_size(
                                        rect.left_bottom() - vec2(0.0, 2.0),
                                        vec2(rect.width(), 2.0),
                                    ),
                                    0.0,
                                    ACC,
                                );
                            }
                            let show_x = tw >= 70.0 || cur || resp.hovered();
                            let right = if show_x { 20.0 } else { 4.0 };
                            let tr = Rect::from_min_max(
                                rect.min + vec2(8.0, 0.0),
                                pos2(rect.max.x - right, rect.max.y),
                            );
                            ui.painter_at(tr).text(
                                pos2(tr.min.x, rect.center().y),
                                Align2::LEFT_CENTER,
                                &self.data.tabs[i].name,
                                FontId::proportional(13.0),
                                if dark { Color32::from_gray(230) } else { INK },
                            );
                            if resp.drag_started() {
                                switch = Some(i);
                            }
                            if resp.dragged() || resp.drag_stopped() {
                                drag_from = Some(i);
                            }
                            dropped |= resp.drag_stopped();
                            if resp.clicked() {
                                switch = Some(i);
                            }
                            if resp.double_clicked() {
                                self.renaming = Some(i);
                            }
                            if resp.middle_clicked() {
                                close = Some(i);
                            }
                            if show_x {
                                let xr = Rect::from_min_size(
                                    pos2(rect.max.x - 20.0, rect.min.y),
                                    vec2(20.0, 24.0),
                                );
                                let xresp = ui.interact(xr, Id::new(("tx", i)), Sense::click());
                                p.text(
                                    xr.center(),
                                    Align2::CENTER_CENTER,
                                    "×",
                                    FontId::proportional(14.0),
                                    if xresp.hovered() {
                                        INK
                                    } else {
                                        Color32::from_gray(130)
                                    },
                                );
                                if xresp.clicked() {
                                    close = Some(i);
                                    switch = None;
                                }
                            }
                        }
                        // drag a tab sideways to reorder
                        if let (Some(from), Some(px)) =
                            (drag_from, ctx.pointer_latest_pos().map(|p| p.x))
                        {
                            let to = rects
                                .iter()
                                .enumerate()
                                .min_by(|a, b| {
                                    (a.1.center().x - px)
                                        .abs()
                                        .total_cmp(&(b.1.center().x - px).abs())
                                })
                                .map(|(k, _)| k)
                                .unwrap_or(from);
                            if dropped {
                                reorder = Some((from, to));
                            } else if to != from {
                                let x = if to > from {
                                    rects[to].max.x
                                } else {
                                    rects[to].min.x
                                };
                                ui.painter().line_segment(
                                    [pos2(x, strip.min.y), pos2(x, strip.max.y)],
                                    Stroke::new(2.0_f32, ACC),
                                );
                            }
                        }
                    });
                if ui.button("+").clicked() {
                    add = true;
                }
                if !self.closed.is_empty() && ui.button("Reopen").clicked() {
                    reopen = true;
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Settings").clicked() {
                        self.settings_open = !self.settings_open;
                    }
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
                if let Some((f, t)) = reorder {
                    if f != t && f < self.data.tabs.len() && t < self.data.tabs.len() {
                        let tab = self.data.tabs.remove(f);
                        self.data.tabs.insert(t, tab);
                        let c = self.data.cur;
                        self.data.cur = if c == f {
                            t
                        } else if f < c && t >= c {
                            c - 1
                        } else if f > c && t <= c {
                            c + 1
                        } else {
                            c
                        };
                        self.renaming = None;
                        self.dirty = true;
                    }
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
                    if !(t.notes.is_empty()
                        && t.images.is_empty()
                        && t.pins.is_empty()
                        && t.strokes.is_empty())
                    {
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

    fn search_ui(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| {
            i.consume_key(Modifiers::COMMAND, Key::K) || i.consume_key(Modifiers::COMMAND, Key::F)
        }) {
            self.search = Some(String::new());
        }
        let mut pick: Option<(usize, Option<u64>)> = None;
        let mut close = false;
        if let Some(q) = self.search.as_mut() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                close = true;
            }
            egui::Window::new("search")
                .title_bar(false)
                .resizable(false)
                .anchor(Align2::CENTER_TOP, vec2(0.0, 60.0))
                .show(ctx, |ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(q)
                            .hint_text("Search all boards")
                            .desired_width(380.0),
                    );
                    r.request_focus();
                    let ql = q.to_lowercase();
                    if ql.is_empty() {
                        return;
                    }
                    let mut hits: Vec<(usize, Option<u64>, String)> = Vec::new();
                    for (ti, t) in self.data.tabs.iter().enumerate() {
                        if t.name.to_lowercase().contains(&ql) {
                            hits.push((ti, None, format!("Board: {}", t.name)));
                        }
                        for n in &t.notes {
                            if hits.len() >= 40 {
                                break;
                            }
                            if n.text.to_lowercase().contains(&ql) {
                                let snip: String = n
                                    .text
                                    .chars()
                                    .map(|c| if c == '\n' { ' ' } else { c })
                                    .take(60)
                                    .collect();
                                hits.push((ti, Some(n.id), format!("{} · {}", t.name, snip)));
                            }
                        }
                    }
                    if hits.is_empty() {
                        ui.weak("No results");
                    }
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for (ti, id, label) in &hits {
                                if ui.selectable_label(false, label).clicked() {
                                    pick = Some((*ti, *id));
                                }
                            }
                        });
                    if ui.input(|i| i.key_pressed(Key::Enter)) {
                        if let Some((ti, id, _)) = hits.first() {
                            pick = Some((*ti, *id));
                        }
                    }
                });
        }
        if let Some((ti, id)) = pick {
            self.data.cur = ti;
            self.goto = id;
            self.selected.clear();
            self.dirty = true;
            close = true;
        }
        if close {
            self.search = None;
        }
    }

    fn board(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();
        let bg = ui.interact(rect, Id::new("board"), Sense::click_and_drag());
        let bold = FontFamily::Name("bold".into());
        let dark = self.data.cfg.dark;
        let mini = self.data.cfg.mini;
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
            self.cur_ink = None;
        }
        let draw = tool == Tool::Draw && !esc;
        if self.hist_key != (self.data.cur, self.data.tabs.len()) {
            self.hist.clear();
            self.redo.clear();
            self.hist_key = (self.data.cur, self.data.tabs.len());
        }
        let (undo, redo) = if kb_free {
            ctx.input_mut(|i| {
                let r = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
                    || i.consume_key(Modifiers::COMMAND, Key::Y);
                (!r && i.consume_key(Modifiers::COMMAND, Key::Z), r)
            })
        } else {
            (false, false)
        };
        if kb_free && draw {
            let (lo, hi) = ctx.input(|i| {
                (
                    i.key_pressed(Key::OpenBracket),
                    i.key_pressed(Key::CloseBracket),
                )
            });
            if lo {
                self.brush.size = (self.brush.size * 0.8).max(1.0);
            }
            if hi {
                self.brush.size = (self.brush.size * 1.25).min(80.0);
            }
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

        if undo {
            if let Some(prev) = self.hist.pop() {
                self.redo.push(std::mem::replace(&mut tab.strokes, prev));
                dirty = true;
            }
        }
        if redo {
            if let Some(next) = self.redo.pop() {
                self.hist.push(std::mem::replace(&mut tab.strokes, next));
                dirty = true;
            }
        }
        if let Some(gid) = self.goto.take() {
            if let Some(n) = tab.notes.iter().find(|n| n.id == gid) {
                let c = vec2(n.x + n.w / 2.0, n.y + n.h / 2.0) * tab.zoom;
                tab.pan = [rect.width() / 2.0 - c.x, rect.height() / 2.0 - c.y];
                sel = HashSet::from([gid]);
                dirty = true;
            }
        }
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
            } else if draw && !space {
                if let Some(sp) = bg.interact_pointer_pos() {
                    let w = to_world(sp);
                    self.hist.push(tab.strokes.clone());
                    self.redo.clear();
                    if self.brush.kind == Kind::Eraser {
                        erase(&mut tab.strokes, w, eraser_r(&self.brush));
                    } else {
                        tab.strokes.push(Ink {
                            pts: vec![[w.x, w.y]],
                            color: self.brush.color,
                            size: self.brush.size,
                            alpha: (self.brush.opacity as f32 * 2.55) as u8,
                            cal: self.brush.kind == Kind::Cal,
                        });
                    }
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

        // drawing: pen, calligraphy, partial eraser
        if draw && !space {
            let er = eraser_r(&self.brush);
            let step = (self.brush.size * 0.4).clamp(1.5, 6.0);
            if bg.drag_started_by(PointerButton::Primary) {
                self.hist.push(tab.strokes.clone());
                if self.hist.len() > 60 {
                    self.hist.remove(0);
                }
                self.redo.clear();
                if let Some(s) = ctx.input(|i| i.pointer.press_origin()).map(to_world) {
                    if self.brush.kind == Kind::Eraser {
                        self.last_e = Some(s);
                        dirty |= erase(&mut tab.strokes, s, er);
                    } else {
                        let mut pts = Vec::new();
                        add_pts(&mut pts, s, step);
                        self.cur_ink = Some(Ink {
                            pts,
                            color: self.brush.color,
                            size: self.brush.size,
                            alpha: (self.brush.opacity as f32 * 2.55) as u8,
                            cal: self.brush.kind == Kind::Cal,
                        });
                    }
                }
            }
            if bg.dragged_by(PointerButton::Primary) {
                if let Some(now) = ctx.pointer_latest_pos().map(to_world) {
                    if self.brush.kind == Kind::Eraser {
                        if let Some(l) = self.last_e {
                            for q in path_steps(l, now, er * 0.5) {
                                dirty |= erase(&mut tab.strokes, q, er);
                            }
                        }
                        self.last_e = Some(now);
                    } else if let Some(ink) = &mut self.cur_ink {
                        add_pts(&mut ink.pts, now, step);
                    }
                }
            }
            if bg.drag_stopped_by(PointerButton::Primary) {
                if let Some(ink) = self.cur_ink.take() {
                    tab.strokes.push(ink);
                    dirty = true;
                }
                self.last_e = None;
            }
        }
        if draw {
            bg.context_menu(|ui| {
                let b = &mut self.brush;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut b.kind, Kind::Pen, "Pen");
                    ui.selectable_value(&mut b.kind, Kind::Cal, "Calligraphy");
                    ui.selectable_value(&mut b.kind, Kind::Eraser, "Eraser");
                });
                ui.horizontal(|ui| {
                    for c in PALETTE {
                        let col = Color32::from_rgb(c[0], c[1], c[2]);
                        let btn =
                            ui.add(egui::Button::new("").fill(col).min_size(vec2(20.0, 20.0)));
                        if btn.clicked() {
                            b.color = c;
                            if b.kind == Kind::Eraser {
                                b.kind = Kind::Pen;
                            }
                        }
                    }
                    ui.color_edit_button_srgb(&mut b.color);
                });
                ui.add(egui::Slider::new(&mut b.size, 1.0..=80.0).text("Size"));
                ui.add(egui::Slider::new(&mut b.opacity, 5..=100).text("Opacity %"));
            });
        }

        // dotted background
        let painter = ui.painter_at(rect);
        let sp = 24.0 * tab.zoom;
        if sp >= 12.0 && !draw {
            let ox = tab.pan[0].rem_euclid(sp);
            let oy = tab.pan[1].rem_euclid(sp);
            let mut x = rect.min.x + ox;
            while x < rect.max.x {
                let mut y = rect.min.y + oy;
                while y < rect.max.y {
                    painter.circle_filled(
                        pos2(x, y),
                        1.0,
                        if dark {
                            Color32::from_rgb(0x3a, 0x3a, 0x44)
                        } else {
                            DOT
                        },
                    );
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
            let at = to_world(hover.unwrap_or(rect.center()));
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
        {
            let org = view.origin.to_vec2() + view.pan;
            for k in &tab.strokes {
                paint_ink(&painter, k, org, view.z, rect);
            }
            if let Some(k) = &self.cur_ink {
                paint_ink(&painter, k, org, view.z, rect);
            }
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
                    Stroke::new(1.0_f32, ACC),
                ));
            } else {
                self.marquee = None;
            }
        }

        if mini {
            let mr = Rect::from_min_size(
                pos2(rect.max.x - 192.0, rect.max.y - 132.0),
                vec2(180.0, 120.0),
            );
            let vt = to_world(rect.min);
            let mut b = Rect::from_min_size(pos2(vt.x, vt.y), rect.size() / view.z);
            let vp = b;
            for n in &tab.notes {
                b = b.union(Rect::from_min_size(pos2(n.x, n.y), vec2(n.w, n.h)));
            }
            for m in &tab.images {
                b = b.union(Rect::from_min_size(pos2(m.x, m.y), vec2(m.w, m.h)));
            }
            for k in &tab.strokes {
                for q in k.pts.iter().step_by(8) {
                    b.extend_with(pos2(q[0], q[1]));
                }
            }
            let b = b.expand(40.0);
            let sc = (mr.width() / b.width()).min(mr.height() / b.height());
            let map = |w: Pos2| mr.center() + (w - b.center()) * sc;
            let mp = ui.painter_at(mr);
            mp.rect_filled(
                mr,
                4.0,
                if dark {
                    Color32::from_rgba_unmultiplied(40, 40, 48, 235)
                } else {
                    Color32::from_white_alpha(235)
                },
            );
            for m in &tab.images {
                let r = Rect::from_min_size(map(pos2(m.x, m.y)), vec2(m.w, m.h) * sc);
                mp.rect_filled(r, 0.0, Color32::from_gray(190));
            }
            for n in &tab.notes {
                let r = Rect::from_min_size(map(pos2(n.x, n.y)), vec2(n.w, n.h) * sc);
                mp.rect_filled(r, 0.0, NOTE);
            }
            for k in &tab.strokes {
                let col = Color32::from_rgb(k.color[0], k.color[1], k.color[2]);
                let pts: Vec<Pos2> = k
                    .pts
                    .iter()
                    .step_by(8)
                    .map(|q| map(pos2(q[0], q[1])))
                    .collect();
                if pts.len() > 1 {
                    mp.add(egui::Shape::line(pts, Stroke::new(1.0_f32, col)));
                }
            }
            let vr = Rect::from_min_max(map(vp.min), map(vp.max));
            mp.add(egui::Shape::closed_line(
                vec![
                    vr.left_top(),
                    vr.right_top(),
                    vr.right_bottom(),
                    vr.left_bottom(),
                ],
                Stroke::new(1.5_f32, ACC),
            ));
            mp.add(egui::Shape::closed_line(
                vec![
                    mr.left_top(),
                    mr.right_top(),
                    mr.right_bottom(),
                    mr.left_bottom(),
                ],
                Stroke::new(1.0_f32, Color32::from_gray(170)),
            ));
            let mresp = ui.interact(mr, Id::new("mini"), Sense::click_and_drag());
            if mresp.clicked() || mresp.dragged() {
                if let Some(pp) = mresp.interact_pointer_pos() {
                    let w = b.center() + (pp - mr.center()) / sc;
                    tab.pan = [
                        rect.width() / 2.0 - w.x * view.z,
                        rect.height() / 2.0 - w.y * view.z,
                    ];
                    dirty = true;
                }
            }
        }
        if draw && !space {
            if let Some(h) = hover {
                let rad = if self.brush.kind == Kind::Eraser {
                    eraser_r(&self.brush)
                } else {
                    self.brush.size / 2.0
                } * view.z;
                painter.circle_stroke(
                    h,
                    rad.max(2.0),
                    Stroke::new(1.0_f32, Color32::from_black_alpha(170)),
                );
                painter.text(
                    h + vec2(rad.max(2.0) + 8.0, 10.0),
                    Align2::LEFT_CENTER,
                    format!("{:.0}px · {}%", self.brush.size, self.brush.opacity),
                    FontId::proportional(11.0),
                    Color32::from_black_alpha(150),
                );
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        }
        if tab.notes.is_empty()
            && tab.images.is_empty()
            && tab.pins.is_empty()
            && tab.strokes.is_empty()
        {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Just start typing to add a note, or paste / drop an image",
                FontId::proportional(15.0),
                if dark {
                    Color32::from_white_alpha(110)
                } else {
                    Color32::from_black_alpha(100)
                },
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
        self.search_ui(ctx);
        self.settings_ui(ctx);
        self.top_bar(ctx);
        egui::TopBottomPanel::top("tools").show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (t, l) in [
                    (Tool::Select, "Select"),
                    (Tool::Pin, "Pin"),
                    (Tool::Hand, "Hand"),
                    (Tool::Draw, "Draw"),
                ] {
                    ui.selectable_value(&mut self.tool, t, l);
                }
                ui.separator();
                if ui.toggle_value(&mut self.data.cfg.mini, "Map").changed() {
                    self.dirty = true;
                }
                ui.separator();
                if self.tool == Tool::Draw {
                    ui.weak("Right-click for brush menu · [ ] size · Ctrl+Z / Ctrl+Y undo · Esc exits");
                } else {
                    ui.weak("Ctrl+click or drag a box to multi-select · right-click a pin to tie strings");
                }
            });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(if self.tool == Tool::Draw {
                Color32::WHITE
            } else if self.data.cfg.dark {
                Color32::from_rgb(0x1c, 0x1c, 0x22)
            } else {
                BG
            }))
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

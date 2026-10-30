use crate::stabilizer::Stabilizer;
use crate::{
    document::{Document, Point, Stroke},
    geometry::{intersect_plane, Camera, Plane},
    input::{PenInput, PenSample, Phase},
    renderer::SceneRenderer,
};
use crate::{editing::Edit, gizmo::Mode};
use eframe::egui::{self, Color32, Key, PointerButton, Pos2, Rect, RichText, Vec2};
use glam::Vec3;
use std::collections::HashSet;
#[path = "selection.rs"]
mod selection;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
enum Pending {
    New,
    Open,
    Close,
}

pub struct SketchApp {
    renderer: SceneRenderer,
    pen: Option<PenInput>,
    input_status: String,
    doc: Document,
    camera: Camera,
    plane: Plane,
    offset: f32,
    color: [f32; 3],
    radius: f32,
    pressure_curve: f32,
    stabilization: f32,
    stabilizer: Stabilizer,
    eraser: bool,
    select_mode: bool,
    selected: HashSet<u64>,
    transform_mode: Mode,
    transform_values: [f32; 3],
    scale_value: f32,
    drag: Option<selection::Drag>,
    active: Option<u64>,
    mouse_down: bool,
    next_id: u64,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    file: Option<PathBuf>,
    dirty: bool,
    status: String,
    last_pen: Option<PenSample>,
    last_pen_at: Option<Instant>,
    pen_previous: Option<Pos2>,
    samples: u64,
    last_batch: usize,
    pressure_history: std::collections::VecDeque<f32>,
    pending: Option<Pending>,
    confirm: bool,
    show_help: bool,
    frame_ms: f32,
    verification_frame: Option<u32>,
}
impl SketchApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = Color32::from_rgb(20, 27, 37);
        style.visuals.window_fill = Color32::from_rgb(25, 34, 46);
        style.visuals.selection.bg_fill = Color32::from_rgb(27, 105, 111);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(32, 124, 132);
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        cc.egui_ctx.set_style(style);
        let (pen, input_status) = match PenInput::install(cc) {
            Ok(p) => (Some(p), "Windows Ink listener ready".into()),
            Err(e) => (None, e),
        };
        let renderer = SceneRenderer::new(
            cc.wgpu_render_state
                .clone()
                .expect("wgpu renderer required"),
        );
        let mut app = Self {
            renderer,
            pen,
            input_status,
            doc: Document::default(),
            camera: Camera::default(),
            plane: Plane::Front,
            offset: 0.0,
            color: [0.17, 0.82, 0.74],
            radius: 0.025,
            pressure_curve: 1.0,
            stabilization: 0.35,
            stabilizer: Stabilizer::default(),
            eraser: false,
            select_mode: false,
            selected: HashSet::new(),
            transform_mode: Mode::Move,
            transform_values: [0.0; 3],
            scale_value: 1.0,
            drag: None,
            active: None,
            mouse_down: false,
            next_id: 1,
            undo: Vec::new(),
            redo: Vec::new(),
            file: None,
            dirty: false,
            status: "Ready. Draw on the highlighted plane.".into(),
            last_pen: None,
            last_pen_at: None,
            pen_previous: None,
            samples: 0,
            last_batch: 0,
            pressure_history: Default::default(),
            pending: None,
            confirm: false,
            show_help: false,
            frame_ms: 0.0,
            verification_frame: std::env::args().any(|a| a == "--verify").then_some(0),
        };
        if std::env::args().any(|a| a == "--demo") {
            app.demo();
        }
        app
    }
    fn finish(&mut self) {
        self.commit_transform();
        let tail = self.stabilizer.finish();
        if let Some(id) = self.active {
            if let Some(stroke) = self.doc.strokes.last_mut().filter(|s| s.id == id) {
                for point in tail {
                    if stroke.points.last().is_none_or(|p| {
                        p.position != point.position || p.pressure != point.pressure
                    }) {
                        stroke.points.push(point);
                    }
                }
            }
        }
        if let Some(id) = self.active.take() {
            if let Some(s) = self.doc.strokes.last().filter(|s| s.id == id) {
                self.undo.push(Edit::Add(s.clone()));
                self.redo.clear();
                self.dirty = true;
            }
        }
    }
    fn cancel(&mut self) {
        self.cancel_transform();
        self.stabilizer.reset();
        self.mouse_down = false;
        if let Some(id) = self.active.take() {
            self.doc.strokes.retain(|s| s.id != id);
        }
    }
    fn add_point(&mut self, position: Pos2, pressure: f32, rect: Rect, begin: bool) {
        let ndc = glam::vec2(
            (position.x - rect.left()) / rect.width() * 2.0 - 1.0,
            1.0 - (position.y - rect.top()) / rect.height() * 2.0,
        );
        let (origin, direction) = self.camera.ray(ndc, rect.aspect_ratio());
        let Some(world) = intersect_plane(origin, direction, self.plane, self.offset) else {
            self.status = "Plane is edge-on. Use Align view or rotate the camera.".into();
            return;
        };
        if world.abs().max_element() > 10000.0 {
            return;
        }
        if begin {
            self.finish();
            let id = self.next_id;
            self.next_id += 1;
            self.active = Some(id);
            self.doc.strokes.push(Stroke {
                id,
                color: self.color,
                radius: self.radius,
                points: Vec::new(),
            });
        }
        let Some(id) = self.active else {
            return;
        };
        let pressure = pressure.clamp(0.0, 1.0).powf(self.pressure_curve);
        let clip = self.camera.matrix(rect.aspect_ratio()) * world.extend(1.0);
        let world_per_pixel = 2.0 * 22.5_f32.to_radians().tan() * clip.w / rect.height();
        let point = self.stabilizer.push(
            Point {
                position: world,
                pressure,
            },
            self.stabilization * 24.0 * world_per_pixel,
        );
        if let Some(stroke) = self.doc.strokes.last_mut().filter(|s| s.id == id) {
            if stroke.points.last().is_none_or(|p| {
                p.position.distance(point.position) > 0.001
                    || (p.pressure - point.pressure).abs() > 0.025
            }) {
                stroke.points.push(point);
            }
        }
    }
    fn erase(&mut self, position: Pos2, rect: Rect) {
        self.finish();
        let matrix = self.camera.matrix(rect.aspect_ratio());
        let project = |point: Vec3| -> Option<Pos2> {
            let clip = matrix * point.extend(1.0);
            if clip.w <= 0.0 {
                return None;
            }
            let ndc = clip.truncate() / clip.w;
            if !(0.0..=1.0).contains(&ndc.z) {
                return None;
            }
            Some(egui::pos2(
                rect.left() + (ndc.x + 1.0) * 0.5 * rect.width(),
                rect.top() + (1.0 - ndc.y) * 0.5 * rect.height(),
            ))
        };
        let index = self
            .doc
            .strokes
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, s)| {
                let hit = s.points.windows(2).any(|pair| {
                    let (Some(a), Some(b)) = (project(pair[0].position), project(pair[1].position))
                    else {
                        return false;
                    };
                    let ab = b - a;
                    let t = ((position - a).dot(ab) / ab.length_sq().max(0.0001)).clamp(0.0, 1.0);
                    position.distance(a + ab * t) < 12.0
                }) || (s.points.len() == 1
                    && project(s.points[0].position).is_some_and(|p| p.distance(position) < 12.0));
                hit.then_some(i)
            });
        if let Some(i) = index {
            let stroke = self.doc.strokes.remove(i);
            self.undo.push(Edit::Remove(i, stroke));
            self.redo.clear();
            self.dirty = true;
        }
    }
    fn undo(&mut self) {
        self.finish();
        if let Some(edit) = self.undo.pop() {
            edit.apply(&mut self.doc, false);
            self.redo.push(edit);
            self.dirty = true;
            self.renderer.invalidate();
            self.sync_selection();
        }
    }
    fn redo(&mut self) {
        self.finish();
        if let Some(edit) = self.redo.pop() {
            edit.apply(&mut self.doc, true);
            self.undo.push(edit);
            self.dirty = true;
            self.renderer.invalidate();
            self.sync_selection();
        }
    }
    fn save(&mut self, save_as: bool) -> bool {
        self.finish();
        let path = if save_as || self.file.is_none() {
            rfd::FileDialog::new()
                .add_filter("Spatial Sketch", &["json"])
                .set_file_name("drawing.sketch.json")
                .save_file()
        } else {
            self.file.clone()
        };
        let Some(path) = path else {
            return false;
        };
        let result = (|| -> Result<(), String> {
            let bytes = serde_json::to_vec_pretty(&self.doc).map_err(|e| e.to_string())?;
            let parent = path.parent().ok_or("Invalid save location")?;
            let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            use std::io::Write;
            temp.write_all(&bytes).map_err(|e| e.to_string())?;
            temp.as_file().sync_all().map_err(|e| e.to_string())?;
            temp.persist(&path).map_err(|e| e.to_string())?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.file = Some(path);
                self.dirty = false;
                true
            }
            Err(e) => {
                self.status = format!("Save failed: {e}");
                false
            }
        }
    }
    fn request(&mut self, pending: Pending) {
        self.finish();
        self.pending = Some(pending);
        if self.dirty {
            self.confirm = true;
        } else {
            self.execute_pending();
        }
    }
    fn execute_pending(&mut self) {
        self.confirm = false;
        match self.pending.take() {
            Some(Pending::New) => {
                self.doc = Document::default();
                self.file = None;
                self.dirty = false;
                self.undo.clear();
                self.redo.clear();
                self.selected.clear();
                self.renderer.invalidate();
                self.status = "New drawing".into();
            }
            Some(Pending::Open) => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Spatial Sketch", &["json"])
                    .pick_file()
                {
                    let result = (|| -> Result<Document, String> {
                        let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                        if file.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
                            return Err("Document exceeds the prototype's 64 MB limit".into());
                        }
                        use std::io::Read;
                        let mut text = String::new();
                        file.read_to_string(&mut text).map_err(|e| e.to_string())?;
                        Document::from_json(&text)
                    })();
                    match result {
                        Ok(doc) => {
                            self.next_id = doc.strokes.iter().map(|s| s.id).max().unwrap_or(0) + 1;
                            self.doc = doc;
                            self.file = Some(path);
                            self.dirty = false;
                            self.undo.clear();
                            self.redo.clear();
                            self.selected.clear();
                            self.renderer.invalidate();
                            self.status = "Drawing opened".into();
                        }
                        Err(e) => self.status = format!("Open failed: {e}"),
                    }
                }
            }
            Some(Pending::Close) => {}
            None => {}
        }
    }
    fn demo(&mut self) {
        for j in 0..3 {
            let mut points = Vec::new();
            for i in 0..160 {
                let t = i as f32 / 159.0 * std::f32::consts::TAU;
                let pos = match j {
                    0 => Vec3::new(t.cos() * 1.6, t.sin() * 1.6, 0.0),
                    1 => Vec3::new(0.0, t.sin() * 1.6, t.cos() * 1.6),
                    _ => Vec3::new(t.cos() * 1.6, 0.0, t.sin() * 1.6),
                };
                points.push(Point {
                    position: pos,
                    pressure: 0.5 + 0.5 * (t * 2.0).sin().abs(),
                });
            }
            self.doc.strokes.push(Stroke {
                id: self.next_id,
                color: [[0.17, 0.82, 0.74], [0.96, 0.55, 0.30], [0.47, 0.58, 0.95]][j],
                radius: 0.035,
                points,
            });
            self.next_id += 1;
        }
        self.status = "Demo scene. New starts a blank drawing.".into();
    }
    fn tools(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(
            RichText::new("WORKSPACE")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.heading("Draw in space");
        ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(!self.eraser && !self.select_mode, "Brush [B]")
                .clicked()
            {
                self.finish();
                self.eraser = false;
                self.select_mode = false;
            }
            if ui
                .selectable_label(self.eraser && !self.select_mode, "Erase [E]")
                .clicked()
            {
                self.finish();
                self.eraser = true;
                self.select_mode = false;
            }
            if ui
                .selectable_label(self.select_mode, "Select [V]")
                .clicked()
            {
                self.finish();
                self.eraser = false;
                self.select_mode = true;
            }
        });
        ui.add_space(6.0);
        if self.select_mode {
            self.selection_panel(ui);
        } else {
            if self.eraser {
                ui.label("Eraser removes whole strokes.");
            }
            ui.label("INK COLOR");
            ui.color_edit_button_rgb(&mut self.color);
            ui.horizontal_wrapped(|ui| {
                for color in [
                    [0.17, 0.82, 0.74],
                    [0.96, 0.55, 0.30],
                    [0.47, 0.58, 0.95],
                    [0.94, 0.80, 0.39],
                    [0.92, 0.94, 0.97],
                ] {
                    let fill = Color32::from_rgb(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                    );
                    if ui.add(egui::Button::new("  ").fill(fill)).clicked() {
                        self.color = color;
                    }
                }
            });
            ui.add(
                egui::Slider::new(&mut self.radius, 0.005..=0.12)
                    .text("Radius")
                    .logarithmic(true),
            );
            ui.add(egui::Slider::new(&mut self.pressure_curve, 0.3..=2.5).text("Pressure"));
            ui.add(egui::Slider::new(&mut self.stabilization, 0.0..=1.0).text("Stabilize").custom_formatter(|v, _| format!("{:.0}%", v*100.0))).on_hover_text("0% gives raw input. Higher values reduce jitter and let the line trail the pen. It catches up on release; pressure is preserved.");
        }
        ui.add_space(12.0);
        ui.separator();
        ui.label(
            RichText::new("DRAWING PLANE")
                .small()
                .color(Color32::from_gray(140)),
        );
        for plane in [Plane::Front, Plane::Side, Plane::Ground] {
            if ui
                .selectable_label(self.plane == plane, plane.label())
                .clicked()
            {
                self.finish();
                self.plane = plane;
            }
        }
        let old = self.offset;
        ui.add(egui::Slider::new(&mut self.offset, -4.0..=4.0).text("Depth"));
        if old != self.offset {
            self.finish();
        }
        if ui.button("Align view to plane [F]").clicked() {
            self.finish();
            self.camera.align(self.plane);
        }
        if ui.button("Reset camera").clicked() {
            self.finish();
            self.camera = Camera::default();
        }
        ui.add_space(12.0);
        ui.separator();
        ui.label(
            RichText::new("NAVIGATION")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.label("Space + drag    Orbit\nSpace + Shift    Pan\nSpace + Alt       Zoom\nRight drag        Orbit\nMiddle drag      Pan\nScroll                Zoom");
        if ui.button("All shortcuts / help").clicked() {
            self.finish();
            self.show_help = true;
        }
    }
    fn diagnostics(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(
            RichText::new("INPUT LAB")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.heading("Tablet diagnostics");
        ui.label(&self.input_status);
        ui.add_space(12.0);
        let recent = self
            .last_pen_at
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2));
        let label = if recent {
            "PEN DETECTED"
        } else {
            "WAITING FOR PEN"
        };
        ui.label(RichText::new(label).color(if recent {
            Color32::from_rgb(76, 220, 190)
        } else {
            Color32::from_gray(150)
        }));
        if let Some(p) = self.last_pen {
            match p.pressure {
                Some(v) => {
                    ui.add(egui::ProgressBar::new(v).text(format!("Pressure  {:.1}%", v * 100.0)));
                }
                None => {
                    ui.label("Pressure: not reported");
                }
            }
            ui.label(format!(
                "Tilt X: {}\nTilt Y: {}",
                p.tilt[0]
                    .map(|v| format!("{v:.0} degrees"))
                    .unwrap_or("not reported".into()),
                p.tilt[1]
                    .map(|v| format!("{v:.0} degrees"))
                    .unwrap_or("not reported".into())
            ));
            ui.label(format!(
                "Contact: {}\nBarrel button: {}\nEraser: {}",
                p.contact, p.barrel, p.eraser
            ));
        } else {
            ui.label("Touch the tablet with your pen.\nMouse strokes use fixed width;\nthey do not count as pen input.");
        }
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 70.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 4.0, Color32::from_rgb(13, 20, 29));
        let points: Vec<Pos2> = self
            .pressure_history
            .iter()
            .enumerate()
            .map(|(i, p)| {
                egui::pos2(
                    rect.left() + i as f32 / 179.0 * rect.width(),
                    rect.bottom() - p * rect.height(),
                )
            })
            .collect();
        if points.len() > 1 {
            ui.painter().add(egui::Shape::line(
                points,
                egui::Stroke::new(1.5_f32, Color32::from_rgb(76, 220, 190)),
            ));
        }
        ui.label(format!(
            "Pen samples: {}\nLast batch: {}",
            self.samples, self.last_batch
        ));
        ui.add_space(12.0);
        ui.separator();
        ui.label(
            RichText::new("SCENE")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.label(format!(
            "{} strokes\n{} control points",
            self.doc.strokes.len(),
            self.doc
                .strokes
                .iter()
                .map(|s| s.points.len())
                .sum::<usize>()
        ));
        ui.label(format!("UI + canvas CPU: {:.2} ms", self.frame_ms));
        ui.label(RichText::new(&self.renderer.adapter).small());
        ui.add_space(12.0);
        ui.separator();
        ui.label("Windows tablet setup");
        ui.label(
            "Enable Windows Ink in your tablet driver. Map the tablet to the intended monitor.",
        );
        ui.label("Wintab and native macOS / Linux pen backends are future work.");
    }
    fn navigate(&mut self, delta: Vec2, pan: bool, zoom: bool, rect: Rect) {
        self.finish();
        if zoom {
            self.camera.distance =
                (self.camera.distance * (delta.y * 0.012).exp()).clamp(0.3, 150.0);
        } else if pan {
            self.camera.pan(glam::vec2(delta.x, delta.y), rect.height());
        } else {
            self.camera.yaw -= delta.x * 0.008;
            self.camera.pitch = (self.camera.pitch + delta.y * 0.008).clamp(-1.55, 1.55);
        }
    }
    fn pointer_action(&mut self, pos: Pos2, pressure: f32, rect: Rect, begin: bool, shift: bool) {
        if self.select_mode {
            self.select_pointer(pos, rect, begin, shift);
        } else if self.eraser {
            self.erase(pos, rect);
        } else {
            self.add_point(pos, pressure, rect, begin);
        }
    }
    fn canvas_input(&mut self, ctx: &egui::Context, rect: Rect, mut pen: Vec<PenSample>) {
        let (space, shift, alt, events, pointer, scroll) = ctx.input(|i| {
            (
                i.key_down(Key::Space),
                i.modifiers.shift,
                i.modifiers.alt,
                i.events.clone(),
                i.pointer.clone(),
                i.smooth_scroll_delta.y,
            )
        });
        if !ctx.input(|i| i.focused) && self.verification_frame.is_none() {
            self.cancel();
            return;
        }
        let blocked = self.confirm || self.show_help;
        let scale = ctx.pixels_per_point();
        for sample in &mut pen {
            sample.position.x /= scale;
            sample.position.y /= scale;
        }
        self.last_batch = pen.len();
        for sample in pen {
            if sample.phase == Phase::Cancel {
                self.cancel();
                self.pen_previous = None;
                continue;
            }
            self.last_pen = Some(sample);
            self.last_pen_at = Some(Instant::now());
            self.samples += 1;
            if let Some(p) = sample.pressure {
                self.pressure_history.push_back(p);
                if self.pressure_history.len() > 180 {
                    self.pressure_history.pop_front();
                }
            }
            let inside = rect.contains(sample.position);
            let previous = self.pen_previous.replace(sample.position);
            if blocked {
                self.finish();
                continue;
            }
            if sample.phase == Phase::Up {
                if self.drag.is_some() {
                    self.select_pointer(sample.position, rect, false, shift);
                }
                self.finish();
                continue;
            }
            if !inside && self.drag.is_none() {
                self.finish();
                continue;
            }
            if space || sample.barrel {
                if sample.contact {
                    if let Some(prev) = previous {
                        self.navigate(sample.position - prev, shift, alt, rect);
                    }
                }
                continue;
            }
            if sample.contact {
                if sample.eraser {
                    self.erase(sample.position, rect);
                } else {
                    self.pointer_action(
                        sample.position,
                        sample.pressure.unwrap_or(1.0),
                        rect,
                        sample.phase == Phase::Down,
                        shift,
                    );
                }
            }
        }
        if blocked {
            return;
        }
        let native_recent = self
            .last_pen_at
            .is_some_and(|t| t.elapsed() < Duration::from_millis(200));
        if native_recent {
            self.mouse_down = false;
        }
        if !native_recent {
            let nav = pointer.button_down(PointerButton::Secondary)
                || pointer.button_down(PointerButton::Middle)
                || (space && pointer.primary_down());
            if nav && pointer.interact_pos().is_some_and(|p| rect.contains(p)) {
                self.mouse_down = false;
                self.navigate(
                    pointer.delta(),
                    shift || pointer.button_down(PointerButton::Middle),
                    alt,
                    rect,
                );
            }
            if !nav && !space {
                for event in events {
                    match event {
                        egui::Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: true,
                            modifiers,
                        } if rect.contains(pos) => {
                            self.mouse_down = true;
                            self.pointer_action(pos, 1.0, rect, true, modifiers.shift);
                        }
                        egui::Event::PointerMoved(pos) if self.mouse_down => {
                            if rect.contains(pos) || self.drag.is_some() {
                                self.pointer_action(pos, 1.0, rect, false, shift);
                            } else {
                                self.mouse_down = false;
                                self.finish();
                            }
                        }
                        egui::Event::PointerButton {
                            pos,
                            button: PointerButton::Primary,
                            pressed: false,
                            ..
                        } => {
                            if self.drag.is_some() {
                                self.select_pointer(pos, rect, false, shift);
                            }
                            self.mouse_down = false;
                            self.finish();
                        }
                        egui::Event::PointerGone => {
                            self.mouse_down = false;
                            self.finish();
                        }
                        _ => {}
                    }
                }
            }
        }
        if pointer.hover_pos().is_some_and(|p| rect.contains(p)) && scroll != 0.0 {
            self.finish();
            self.camera.distance =
                (self.camera.distance * (-scroll * 0.002).exp()).clamp(0.3, 150.0);
        }
        self.sync_selection();
    }
}
impl eframe::App for SketchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let start = Instant::now();
        self.capture_verification(ctx);
        if ctx.input(|i| i.viewport().close_requested()) {
            self.finish();
            if self.dirty {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.request(Pending::Close);
            }
        }
        if !self.confirm && !self.show_help && !ctx.wants_keyboard_input() {
            self.selection_shortcuts(ctx);
            let (mods, b, e, f, z, y, s, o, n) = ctx.input(|i| {
                (
                    i.modifiers,
                    i.key_pressed(Key::B),
                    i.key_pressed(Key::E),
                    i.key_pressed(Key::F),
                    i.key_pressed(Key::Z),
                    i.key_pressed(Key::Y),
                    i.key_pressed(Key::S),
                    i.key_pressed(Key::O),
                    i.key_pressed(Key::N),
                )
            });
            if mods.command {
                if z {
                    if mods.shift {
                        self.redo()
                    } else {
                        self.undo()
                    }
                }
                if y {
                    self.redo()
                }
                if s {
                    self.save(mods.shift);
                }
                if o {
                    self.request(Pending::Open)
                }
                if n {
                    self.request(Pending::New)
                }
            } else {
                if b {
                    self.finish();
                    self.eraser = false;
                    self.select_mode = false;
                }
                if e {
                    self.finish();
                    self.eraser = true;
                    self.select_mode = false;
                }
                if f {
                    self.finish();
                    self.camera.align(self.plane);
                }
            }
        }
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(7.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("SPATIAL / SKETCH")
                        .strong()
                        .size(20.0)
                        .color(Color32::from_rgb(99, 229, 207)),
                );
                ui.label(
                    RichText::new("0.1  /  RUST PROTOTYPE")
                        .small()
                        .color(Color32::from_gray(140)),
                );
                ui.separator();
                let enabled = !self.confirm;
                if ui.add_enabled(enabled, egui::Button::new("New")).clicked() {
                    self.request(Pending::New);
                }
                if ui.add_enabled(enabled, egui::Button::new("Open")).clicked() {
                    self.request(Pending::Open);
                }
                if ui.add_enabled(enabled, egui::Button::new("Save")).clicked() {
                    self.save(false);
                }
                if ui
                    .add_enabled(enabled, egui::Button::new("Save as"))
                    .clicked()
                {
                    self.save(true);
                }
                ui.separator();
                if ui
                    .add_enabled(
                        enabled
                            && (!self.undo.is_empty()
                                || self.active.is_some()
                                || self.drag.is_some()),
                        egui::Button::new("Undo"),
                    )
                    .clicked()
                {
                    self.undo();
                }
                if ui
                    .add_enabled(enabled && !self.redo.is_empty(), egui::Button::new("Redo"))
                    .clicked()
                {
                    self.redo();
                }
                ui.label(if self.dirty {
                    "Unsaved changes"
                } else {
                    "Saved / blank"
                });
            });
            ui.add_space(7.0);
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(self.plane.label());
                });
            });
        });
        egui::SidePanel::left("tools")
            .exact_width(240.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_enabled_ui(!self.confirm, |ui| self.tools(ui));
                });
            });
        egui::SidePanel::right("diagnostics")
            .exact_width(245.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.diagnostics(ui));
            });
        let pen = self.pen.as_ref().map(|p| p.drain()).unwrap_or_default();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                self.verify_canvas(ctx, rect);
                self.canvas_input(ctx, rect, pen);
                let scale = ctx.pixels_per_point();
                if self.select_mode {
                    self.renderer.set_selection(&self.selected);
                } else {
                    self.renderer.set_selection(&HashSet::new());
                }
                let id = self.renderer.render(
                    &self.doc,
                    &self.camera,
                    self.plane,
                    self.offset,
                    [
                        (rect.width() * scale).round() as u32,
                        (rect.height() * scale).round() as u32,
                    ],
                );
                ui.painter().image(
                    id,
                    rect,
                    Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                self.selection_overlay(ui, rect);
                let badge =
                    Rect::from_min_size(rect.min + egui::vec2(20.0, 20.0), egui::vec2(200.0, 32.0));
                ui.painter()
                    .rect_filled(badge, 5.0, Color32::from_black_alpha(110));
                ui.painter().text(
                    badge.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{}   /   {:.1}", self.plane.label(), self.offset),
                    egui::FontId::proportional(13.0),
                    Color32::from_rgb(173, 210, 213),
                );
                if self.doc.strokes.is_empty() {
                    ui.painter().text(
                        rect.center() + egui::vec2(0.0, -35.0),
                        egui::Align2::CENTER_CENTER,
                        "Your next idea starts here.",
                        egui::FontId::proportional(25.0),
                        Color32::from_gray(190),
                    );
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Pen to draw   /   Space + drag to explore",
                        egui::FontId::proportional(14.0),
                        Color32::from_gray(125),
                    );
                }
                if response.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                }
            });
        if self.confirm {
            egui::Window::new("Save your drawing?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("There are unsaved changes.");
                    ui.horizontal(|ui| {
                        if ui.button("Save and continue").clicked() && self.save(false) {
                            let close = matches!(self.pending, Some(Pending::Close));
                            self.execute_pending();
                            if close {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        if ui.button("Discard").clicked() {
                            let close = matches!(self.pending, Some(Pending::Close));
                            self.execute_pending();
                            if close {
                                self.dirty = false;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.pending = None;
                            self.confirm = false;
                        }
                    });
                });
        }
        if self.show_help {
            egui::Window::new("Drawing with a tablet").open(&mut self.show_help).resizable(false).show(ctx,|ui|{ui.label("V: select   Shift-click: toggle stroke selection\nG: move   R: rotate   S: uniform scale (in Select tool)\nDrag the visible handles; Escape cancels a transform.\nCtrl/Cmd+A: select all   Ctrl/Cmd+D: duplicate\nDelete / Backspace: delete selection\n\nB: brush   E: whole-stroke eraser   F: align camera to plane\nCtrl/Cmd+Z: undo   Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y: redo\nCtrl/Cmd+S: save   Ctrl/Cmd+Shift+S: save as\nCtrl/Cmd+O: open   Ctrl/Cmd+N: new\n\nSpace + pen drag: orbit\nSpace + Shift + pen drag: pan\nSpace + Alt + pen drag: zoom\nPen barrel + drag: orbit (when reported by Windows Ink)\nRight mouse drag: orbit   Middle drag: pan   Wheel: zoom\n\nChoose Front, Side, or Ground, then move Depth to place the plane.\nExisting strokes stay in world space when you switch planes.\nEnable Windows Ink in your Wacom / Huion driver for pressure.\nA mouse works too, with fixed pressure.\n\nPrototype limits: no Wintab, curved guides, model export, or autosave.\nSave your work using the Save button.");});
        }
        self.frame_ms = start.elapsed().as_secs_f32() * 1000.0;
        if self.active.is_some() || self.drag.is_some() || self.verification_frame.is_some() {
            ctx.request_repaint_after(Duration::from_millis(8));
        }
    }
}

impl SketchApp {
    /// Deterministic software-input smoke test; never labels synthetic input as hardware input.
    fn verify_canvas(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(frame) = self.verification_frame else {
            return;
        };
        match frame {
            1 => {
                self.stabilization = 0.0;
                self.camera.align(Plane::Front);
                let start = rect.center() - egui::vec2(120.0, 0.0);
                let mut events = vec![egui::Event::PointerButton {
                    pos: start,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }];
                for i in 0..100 {
                    events.push(egui::Event::PointerMoved(
                        start + egui::vec2(i as f32 * 2.4, (i as f32 * 0.08).sin() * 65.0),
                    ));
                }
                events.push(egui::Event::PointerButton {
                    pos: start + egui::vec2(240.0, 0.0),
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                });
                ctx.input_mut(|i| i.events = events);
            }
            2 => {
                assert_eq!(self.doc.strokes.len(), 1, "Mouse event stroke must commit");
                assert!(
                    self.doc.strokes[0].points.len() > 80,
                    "Intermediate mouse samples must survive a single frame"
                );
                self.undo();
                assert!(self.doc.strokes.is_empty());
                self.redo();
                assert_eq!(self.doc.strokes.len(), 1);
                self.erase(rect.center() - egui::vec2(120.0, 0.0), rect);
                assert!(self.doc.strokes.is_empty(), "Whole-stroke eraser");
                self.undo();
                assert_eq!(self.doc.strokes.len(), 1);
                self.doc = Document::default();
                self.undo.clear();
                self.redo.clear();
                self.selected.clear();
                self.renderer.invalidate();
                self.demo();
                let path = std::env::current_dir().unwrap().join("verification");
                std::fs::create_dir_all(&path).unwrap();
                self.file = Some(path.join("smoke.sketch.json"));
                assert!(self.save(false));
                assert!(self.save(false), "Replacing an existing save must work");
                let loaded = Document::from_json(
                    &std::fs::read_to_string(self.file.as_ref().unwrap()).unwrap(),
                )
                .unwrap();
                assert_eq!(loaded.strokes.len(), 3);
                assert_eq!(loaded.strokes[0].points.len(), 160);
                self.doc = loaded;
                self.renderer.invalidate();
                self.camera = Camera::default();
                self.status =
                    "Software smoke checks passed. Physical tablet validation is still required."
                        .into();
                // Clear synthetic events before the regular input handler sees them again.
                ctx.input_mut(|i| i.events.clear());
            }
            3 => self.verify_selection(ctx, rect),
            4 => self.verify_stabilization(rect),
            5 => ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default())),
            120 => panic!("Verification screenshot timed out"),
            _ => {}
        }
        self.verification_frame = Some(frame + 1);
    }
    fn capture_verification(&mut self, ctx: &egui::Context) {
        if self.verification_frame.is_none() {
            return;
        }
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let path = std::env::current_dir().unwrap().join("verification");
            let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer(
                path.join("preview.png"),
                &pixels,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            let report = serde_json::json!({"passed":true,"checks":["single-frame mouse sample order","3D geometry and GPU pipeline","undo/redo","whole-stroke eraser","atomic save","document reload","UI screenshot","Shift multi-selection","axis move and view-plane cancel","rotation and uniform scale handles","group duplicate/delete undo","numeric transform","transformed document reload","stabilized stroke endpoint and undo"],"gpu":self.renderer.adapter,"strokes":self.doc.strokes.len(),"physical_tablet_tested":false});
            std::fs::write(
                path.join("report.json"),
                serde_json::to_string_pretty(&report).unwrap(),
            )
            .unwrap();
            self.dirty = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl SketchApp {
    fn verify_stabilization(&mut self, rect: Rect) {
        let original = self.doc.strokes.clone();
        let selection = self.selected.clone();
        let steps = self.undo.len();
        self.select_mode = false;
        self.stabilization = 0.8;
        let start = rect.center() - egui::vec2(100.0, 60.0);
        let mut endpoint = start;
        for i in 0..100 {
            endpoint = start + egui::vec2(i as f32 * 2.0, if i % 2 == 0 { 3.0 } else { -3.0 });
            self.add_point(endpoint, 0.6, rect, i == 0);
        }
        let ndc = glam::vec2(
            (endpoint.x - rect.left()) / rect.width() * 2.0 - 1.0,
            1.0 - (endpoint.y - rect.top()) / rect.height() * 2.0,
        );
        let (o, d) = self.camera.ray(ndc, rect.aspect_ratio());
        let expected = intersect_plane(o, d, self.plane, self.offset).unwrap();
        self.finish();
        let stroke = self.doc.strokes.last().unwrap();
        assert!(stroke.points.last().unwrap().position.distance(expected) < 0.00001);
        assert_eq!(stroke.points.last().unwrap().pressure, 0.6);
        assert_eq!(self.undo.len(), steps + 1);
        self.undo();
        assert_eq!(self.doc.strokes, original);
        self.redo();
        assert_eq!(self.doc.strokes.len(), original.len() + 1);
        self.undo();
        assert_eq!(self.doc.strokes, original);
        self.selected = selection;
        self.select_mode = true;
        self.stabilization = 0.35;
        assert!(self.save(false));
        self.status =
            "Selection, transforms, stabilization, endpoint, and undo checks passed.".into();
    }
}

//! The interactive app.

mod inspector;
mod viewport;
mod worker;

use anyhow::anyhow;
use egui::{Color32, Rect, RichText, Sense, TextureHandle, TextureOptions, Vec2, pos2};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};
use viewport::{Viewer, Viewport};
use worker::{Job, Level, OnDrop, Update, Worker};
use xpro_core::{
    edit::Edit,
    eval::{Inputs, render},
    graph::{Graph, NodeId},
    image::Srgb8Image,
    node::{Ports as _, default_graph},
};

pub fn run(path: Option<PathBuf>) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // On macOS, eframe also uses the initial title as the name in the
            // menu bar, so the window is renamed after the image file below.
            .with_title("Xpro")
            .with_min_inner_size([640.0, 400.0])
            .with_maximized(true),
        ..Default::default()
    };
    eframe::run_native(
        "Xpro",
        options,
        Box::new(|cc| {
            let document = path.as_deref().map(Document::open);
            Ok(Box::new(App::new(&cc.egui_ctx, document)))
        }),
    )
    .map_err(|error| anyhow!("{error}"))
}

/// An image file being edited.
pub struct Document {
    pub image_path: PathBuf,
    pub edit: Edit,
    /// The edit as it was when opened or last saved, to tell whether it's been changed since.
    baseline: Edit,
    /// What the graph's inputs are bound to.
    pub inputs: Inputs,
    /// The node the viewer shows and the inspector edits.
    pub selected: Option<NodeId>,
}

impl Document {
    /// Opens an image file with its edit file, or with the default graph if it has none.
    pub fn open(image_path: &Path) -> anyhow::Result<Self> {
        let edit = Edit::read_for(image_path)?.unwrap_or_else(|| Edit::new(default_graph()));
        let inputs = edit.inputs(image_path);
        Ok(Self::new(image_path.to_owned(), edit, inputs))
    }

    /// A document for an edit whose graph's inputs are bound to `inputs`. Selects the node an
    /// output node is wired to, or else the last node.
    pub fn new(image_path: PathBuf, edit: Edit, inputs: Inputs) -> Self {
        let graph = &edit.graph;
        let selected = graph
            .nodes()
            .filter(|(_, node)| node.node.is_output())
            .find_map(|(_, node)| node.node.inputs().into_iter().find_map(|input| input.wire))
            .or_else(|| graph.topological_order().pop());
        Self {
            image_path,
            baseline: edit.clone(),
            edit,
            inputs,
            selected,
        }
    }

    /// Whether the edit has changed since it was opened or last saved.
    pub fn is_edited(&self) -> bool {
        self.edit != self.baseline
    }

    /// Saves the edit to its edit file, next to the image file.
    pub fn save(&mut self) -> anyhow::Result<PathBuf> {
        let path = Edit::path_for(&self.image_path);
        self.edit.save(&path)?;
        self.baseline = self.edit.clone();
        Ok(path)
    }
}

pub struct App {
    document: Option<anyhow::Result<Document>>,
    shown: Shown,
    worker: Option<Worker>,
    /// What the newest job given to the worker was for, to know when the graph, the selection
    /// or the GPU's texture size limit changes.
    submitted: Option<(Graph, NodeId, usize)>,
    /// The newest job's generation.
    generation: u64,
    /// The newest generation the worker has finished.
    finished: u64,
    /// A message about the last thing done, such as saving.
    status: Option<String>,
    exporting: Option<mpsc::Receiver<anyhow::Result<Vec<PathBuf>>>>,
    /// The window title last set.
    title: String,
    /// The size and viewport of the image last shown, to keep the zoom and position if the
    /// next image is about the same size, even after an error in between.
    last_shown: Option<(Vec2, Viewport)>,
}

/// What the viewer shows.
enum Shown {
    Nothing,
    Waiting,
    Image(Loaded),
    Failed(String),
}

/// An image at several sizes, so it can be drawn sharply at any zoom.
struct Loaded {
    /// Textures of the image, each with how many times it was halved from full size, largest
    /// first.
    levels: Vec<(u8, TextureHandle)>,
    /// The size of the full image, in pixels.
    full_size: Vec2,
    viewport: Viewport,
    /// The job the levels came from.
    generation: u64,
}

impl App {
    fn new(ctx: &egui::Context, document: Option<anyhow::Result<Document>>) -> Self {
        let worker = document.is_some().then(|| {
            let ctx = ctx.clone();
            Worker::spawn(move || ctx.request_repaint())
        });
        Self {
            document,
            shown: Shown::Nothing,
            worker,
            submitted: None,
            generation: 0,
            finished: 0,
            status: None,
            exporting: None,
            title: String::new(),
            last_shown: None,
        }
    }

    /// Whether the worker has finished everything it was given.
    #[cfg(test)]
    fn is_idle(&self) -> bool {
        self.finished == self.generation && self.exporting.is_none()
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive_updates(&ctx);
        self.receive_export();
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.save();
        }
        self.update_title(&ctx);

        let clicked;
        match &mut self.document {
            None => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label("Open an image file with `xpro IMAGE_FILE`")
                    });
                });
                return;
            }
            Some(Err(error)) => {
                let error = format!("{error:#}");
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.centered_and_justified(|ui| ui.label(error));
                });
                return;
            }
            Some(Ok(document)) => {
                let status = self.status.as_deref();
                let exporting = self.exporting.is_some();
                clicked = egui::Panel::top("toolbar")
                    .show(ui, |ui| toolbar(ui, document, status, exporting))
                    .inner;
                egui::Panel::left("nodes")
                    .resizable(false)
                    .exact_size(180.0)
                    .show(ui, |ui| {
                        ui.add_space(6.0);
                        inspector::node_list(ui, &document.edit.graph, &mut document.selected);
                    });
                egui::Panel::right("inspector")
                    .resizable(false)
                    .exact_size(280.0)
                    .show(ui, |ui| {
                        ui.add_space(6.0);
                        if let Some(id) = document.selected.clone() {
                            inspector::inspector(ui, &mut document.edit.graph, &id);
                        }
                    });
            }
        }
        if clicked.save {
            self.save();
        }
        if clicked.export {
            self.export(&ctx);
        }
        // Changes made this frame are evaluated right away.
        self.submit_job(&ctx);
        if self.status.as_deref() == Some("Saved") && self.save_needed() {
            self.status = None;
        }

        egui::CentralPanel::default().show(ui, |ui| match &mut self.shown {
            Shown::Nothing => {}
            Shown::Waiting => {
                ui.centered_and_justified(egui::Ui::spinner);
            }
            Shown::Image(loaded) => show_image(ui, loaded),
            Shown::Failed(error) => {
                ui.centered_and_justified(|ui| ui.label(error.as_str()));
            }
        });
    }
}

/// What was clicked in the toolbar.
#[derive(Default)]
struct Clicked {
    save: bool,
    export: bool,
}

/// The bar above everything: the image file's name, and buttons to save and export.
fn toolbar(
    ui: &mut egui::Ui,
    document: &Document,
    status: Option<&str>,
    exporting: bool,
) -> Clicked {
    let mut clicked = Clicked::default();
    ui.horizontal(|ui| {
        ui.label(RichText::new(file_name(&document.image_path)).strong());
        if document.is_edited() {
            ui.label(RichText::new("Edited").weak());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clicked.export = ui
                .add_enabled(!exporting, egui::Button::new("Export"))
                .on_hover_text("Write the files the graph's output nodes describe")
                .clicked();
            clicked.save = ui
                .button("Save")
                .on_hover_text("Save the edit file (⌘S)")
                .clicked();
            if let Some(status) = status {
                ui.label(RichText::new(status).weak());
            }
        });
    });
    clicked
}

impl App {
    /// Writes the files the graph's output nodes describe, in the background.
    fn export(&mut self, ctx: &egui::Context) {
        let Some(Ok(document)) = &self.document else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let graph = document.edit.graph.clone();
        let inputs = document.inputs.clone();
        let dir = document
            .image_path
            .parent()
            .unwrap_or(Path::new(""))
            .to_owned();
        let ctx = ctx.clone();
        thread::spawn(move || {
            // Even if rendering panics, so the app notices that it stopped.
            let _guard = OnDrop(|| ctx.request_repaint());
            let _ = sender.send(render(&graph, &inputs, &dir));
        });
        self.exporting = Some(receiver);
        self.status = Some("Exporting…".to_owned());
    }

    fn save(&mut self) {
        if let Some(Ok(document)) = &mut self.document {
            self.status = Some(match document.save() {
                Ok(_) => "Saved".to_owned(),
                Err(error) => format!("{error:#}"),
            });
        }
    }

    /// Whether the open edit has unsaved changes.
    fn save_needed(&self) -> bool {
        matches!(&self.document, Some(Ok(document)) if document.is_edited())
    }

    /// Names the window after the image file, noting unsaved changes.
    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match &self.document {
            Some(Ok(document)) if document.is_edited() => {
                format!("{} — Edited", file_name(&document.image_path))
            }
            Some(Ok(document)) => file_name(&document.image_path),
            _ => "Xpro".to_owned(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    /// Gives the worker a new job if the graph or the selected node changed.
    fn submit_job(&mut self, ctx: &egui::Context) {
        let (Some(Ok(document)), Some(worker)) = (&self.document, &mut self.worker) else {
            return;
        };
        let Some(selected) = &document.selected else {
            return;
        };
        let max_texture_side = ctx.input(|input| input.max_texture_side);
        let current = (
            document.edit.graph.clone(),
            selected.clone(),
            max_texture_side,
        );
        if self.submitted.as_ref() == Some(&current) {
            return;
        }
        self.generation += 1;
        worker.submit(Job {
            generation: self.generation,
            graph: current.0.clone(),
            inputs: document.inputs.clone(),
            node: current.1.clone(),
            max_texture_side,
        });
        self.submitted = Some(current);
        if matches!(self.shown, Shown::Nothing | Shown::Failed(_)) {
            self.shown = Shown::Waiting;
        }
    }

    /// Shows what the worker has finished.
    fn receive_updates(&mut self, ctx: &egui::Context) {
        loop {
            let Some(worker) = &self.worker else {
                return;
            };
            let update = match worker.updates.try_recv() {
                Ok(update) => update,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.shown = Shown::Failed("Evaluating crashed".to_owned());
                    self.finished = self.generation;
                    self.worker = None;
                    return;
                }
            };
            let shown_generation = match &self.shown {
                Shown::Image(loaded) => loaded.generation,
                _ => 0,
            };
            match update {
                // Anything newer than what's shown is better, even if it's not the newest job,
                // so the image keeps up while a slider is dragged.
                Update::Level { generation, level } if generation >= shown_generation => {
                    self.show_level(ctx, generation, level);
                }
                Update::Failed { generation, error } if generation >= shown_generation => {
                    if let Shown::Image(old) = &self.shown {
                        self.last_shown = Some((old.full_size, old.viewport));
                    }
                    self.shown = Shown::Failed(format!("{error:#}"));
                }
                Update::Done { generation } => self.finished = self.finished.max(generation),
                Update::Level { .. } | Update::Failed { .. } => {}
            }
        }
    }

    fn show_level(&mut self, ctx: &egui::Context, generation: u64, level: Level) {
        let Level {
            image,
            halvings,
            full_size,
        } = level;
        let texture = upload(ctx, &image, generation, halvings);
        let full_size = Vec2::new(full_size[0] as f32, full_size[1] as f32);
        match &mut self.shown {
            Shown::Image(loaded) if loaded.generation == generation => {
                loaded.levels.retain(|(h, _)| *h != halvings);
                let position = loaded.levels.partition_point(|(h, _)| *h < halvings);
                loaded.levels.insert(position, (halvings, texture));
                // Full-size results come after previews, and know the size exactly, even when
                // the full-size level itself is too big for the GPU.
                loaded.full_size = full_size;
            }
            shown => {
                if let Shown::Image(old) = shown {
                    self.last_shown = Some((old.full_size, old.viewport));
                }
                // Keep the zoom and position if the image is about the same size.
                let viewport = match self.last_shown {
                    Some((size, viewport)) if (size - full_size).abs().max_elem() <= 8.0 => {
                        viewport
                    }
                    _ => Viewport::Fit,
                };
                *shown = Shown::Image(Loaded {
                    levels: vec![(halvings, texture)],
                    full_size,
                    viewport,
                    generation,
                });
            }
        }
    }

    fn receive_export(&mut self) {
        let Some(receiver) = &self.exporting else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(anyhow!("Exporting crashed")),
        };
        self.status = Some(match result {
            Ok(paths) => {
                let names: Vec<String> = paths.iter().map(|path| file_name(path)).collect();
                if names.is_empty() {
                    "Nothing to export".to_owned()
                } else {
                    format!("Exported {}", names.join(", "))
                }
            }
            Err(error) => format!("{error:#}"),
        });
        self.exporting = None;
    }
}

fn upload(ctx: &egui::Context, image: &Srgb8Image, generation: u64, halvings: u8) -> TextureHandle {
    let image =
        egui::ColorImage::from_rgb([image.width, image.height], image.pixels.as_flattened());
    ctx.load_texture(
        format!("image {generation} {halvings}"),
        image,
        TextureOptions::LINEAR,
    )
}

/// Shows the image. Pinching zooms, swiping with two fingers pans, and double-clicking switches
/// between fitting and 100%.
fn show_image(ui: &mut egui::Ui, loaded: &mut Loaded) {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click());
    let viewer = Viewer {
        rect,
        pixels_per_point: ui.pixels_per_point(),
        image: loaded.full_size,
    };
    // Re-apply the bounds every frame, since the window may have been resized.
    let mut viewport = viewer.clamp(loaded.viewport);
    if response.hovered() {
        let (zoom, scroll, pointer) = ui.input(|input| {
            (
                input.zoom_delta(),
                input.smooth_scroll_delta,
                input.pointer.hover_pos(),
            )
        });
        if zoom != 1.0 {
            let anchor = pointer.unwrap_or(rect.center());
            viewport = viewer.zoom(viewport, zoom, anchor);
        }
        if scroll != Vec2::ZERO {
            viewport = viewer.pan(viewport, scroll);
        }
    }
    if response.double_clicked() {
        viewport = match viewport {
            Viewport::Fit => {
                let anchor = response.interact_pointer_pos().unwrap_or(rect.center());
                viewer.zoom_to(viewport, viewer.actual_scale(), anchor)
            }
            Viewport::Zoomed { .. } => Viewport::Fit,
        };
    }
    loaded.viewport = viewport;

    // Line the image up with the screen's pixels, so that at 100% each image pixel lands on
    // exactly one screen pixel instead of blurring across two.
    let ppp = viewer.pixels_per_point;
    let image_rect = viewer.image_rect(viewport);
    let image_rect = Rect::from_min_size(
        ((image_rect.min.to_vec2() * ppp).round() / ppp).to_pos2(),
        image_rect.size(),
    );
    let widths: Vec<usize> = loaded
        .levels
        .iter()
        .map(|(_, level)| level.size()[0])
        .collect();
    let (_, level) = &loaded.levels[sharpest_needed(&widths, image_rect.width() * ppp)];
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter_at(rect)
        .image(level.id(), image_rect, uv, Color32::WHITE);
}

/// Picks which level to draw: the smallest that's at least `shown_width` pixels wide, so it's
/// shrunk by less than half and never enlarged, unless even the largest level is too small.
/// `widths` are the levels' widths, largest first.
fn sharpest_needed(widths: &[usize], shown_width: f32) -> usize {
    widths
        .iter()
        .rposition(|&width| width as f32 >= shown_width)
        .unwrap_or(0)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod ui_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use hegel::{TestCase, generators as gs};

    /// Widths of a full-size image and its successive halvings.
    #[hegel::composite]
    fn gen_widths(tc: &TestCase) -> Vec<usize> {
        let mut width = tc.draw(gs::integers::<usize>().min_value(1).max_value(20_000));
        let count = tc.draw(gs::integers::<usize>().min_value(1).max_value(8));
        let mut widths = vec![width];
        for _ in 1..count {
            width = width.div_ceil(2);
            widths.push(width);
        }
        widths
    }

    #[hegel::test(test_cases = 10_000)]
    fn the_chosen_level_is_never_enlarged_unless_none_is_big_enough(tc: TestCase) {
        let widths = tc.draw(gen_widths());
        let shown = tc.draw(gs::floats::<f32>().min_value(0.0).max_value(40_000.0));
        let chosen = sharpest_needed(&widths, shown);
        assert!(widths[chosen] as f32 >= shown || chosen == 0, "{widths:?}, {shown} → {chosen}");
    }

    #[hegel::test(test_cases = 10_000)]
    fn the_chosen_level_is_the_smallest_big_enough(tc: TestCase) {
        // Drawing a level much bigger than it's shown wastes effort and shimmers.
        let widths = tc.draw(gen_widths());
        let shown = tc.draw(gs::floats::<f32>().min_value(0.0).max_value(40_000.0));
        let chosen = sharpest_needed(&widths, shown);
        if let Some(&smaller) = widths.get(chosen + 1) {
            assert!((smaller as f32) < shown, "{widths:?}, {shown} → {chosen}");
        }
    }
}

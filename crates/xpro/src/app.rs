//! The interactive app.

mod viewport;

use anyhow::{Context as _, anyhow};
use egui::{Color32, Rect, Sense, TextureHandle, TextureOptions, Vec2, pos2};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::Instant,
};
use viewport::{Viewer, Viewport};
use xpro_core::{
    eval::{Input, Inputs, evaluate},
    image::Srgb8Image,
    node::{Node, Ports as _, default_graph},
};

pub fn run(path: Option<PathBuf>) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // On macOS, eframe also uses the initial title as the name in the
            // menu bar, so the window is renamed after the raw file below.
            .with_title("Xpro")
            .with_min_inner_size([480.0, 320.0])
            .with_maximized(true),
        ..Default::default()
    };
    eframe::run_native(
        "Xpro",
        options,
        Box::new(|cc| {
            if let Some(path) = &path {
                cc.egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Title(file_name(path)));
            }
            Ok(Box::new(App {
                image: path.map_or(ImageState::None, ImageState::Pending),
            }))
        }),
    )
    .map_err(|error| anyhow!("{error}"))
}

struct App {
    image: ImageState,
}

/// The state of the image the app shows, from nothing yet to loaded or failed.
enum ImageState {
    None,
    /// Waiting for the first frame, when the GPU's texture size limit is known.
    Pending(PathBuf),
    Loading(mpsc::Receiver<anyhow::Result<Developed>>),
    Loaded(Loaded),
    Failed(anyhow::Error),
}

/// A developed image at several sizes, so it can be drawn sharply at any zoom.
struct Developed {
    /// The image at full size, then halved again and again down to about screen size. Levels
    /// too big for the GPU are left out.
    levels: Vec<Srgb8Image>,
    /// The size of the full image, in pixels.
    full_size: Vec2,
}

struct Loaded {
    /// Textures of [`Developed::levels`], largest first.
    levels: Vec<TextureHandle>,
    /// The size of the full image, in pixels.
    full_size: Vec2,
    viewport: Viewport,
}

/// Levels stop being halved once they're this small on their longest side, which is about as
/// small as the window can show the image.
const SMALLEST_LEVEL: usize = 512;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.update_image(ui.ctx());

        egui::CentralPanel::default().show(ui, |ui| match &mut self.image {
            ImageState::None => {
                ui.centered_and_justified(|ui| ui.label("Open a raw file with `xpro RAW_FILE`"));
            }
            ImageState::Pending(_) | ImageState::Loading(_) => {
                ui.centered_and_justified(egui::Ui::spinner);
            }
            ImageState::Loaded(loaded) => show_image(ui, loaded),
            ImageState::Failed(error) => {
                ui.centered_and_justified(|ui| ui.label(format!("{error:#}")));
            }
        });
    }
}

impl App {
    fn update_image(&mut self, ctx: &egui::Context) {
        match &self.image {
            ImageState::Pending(path) => {
                let max_texture_side = ctx.input(|input| input.max_texture_side);
                let (sender, receiver) = mpsc::channel();
                let path = path.clone();
                let ctx = ctx.clone();
                thread::spawn(move || {
                    // If the app has quit, nobody needs the result.
                    let _ = sender.send(develop_levels(&path, max_texture_side));
                    ctx.request_repaint();
                });
                self.image = ImageState::Loading(receiver);
            }
            ImageState::Loading(receiver) => match receiver.try_recv() {
                Ok(Ok(Developed { levels, full_size })) => {
                    let start = Instant::now();
                    let levels = levels
                        .iter()
                        .enumerate()
                        .map(|(i, image)| {
                            let image = egui::ColorImage::from_rgb(
                                [image.width, image.height],
                                image.pixels.as_flattened(),
                            );
                            ctx.load_texture(format!("image {i}"), image, TextureOptions::LINEAR)
                        })
                        .collect();
                    tracing::info!(elapsed = ?start.elapsed(), "Uploaded textures");
                    self.image = ImageState::Loaded(Loaded {
                        levels,
                        full_size,
                        viewport: Viewport::Fit,
                    });
                }
                Ok(Err(error)) => self.image = ImageState::Failed(error),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.image = ImageState::Failed(anyhow!("Developing the raw file crashed"));
                }
            },
            ImageState::None | ImageState::Loaded(_) | ImageState::Failed(_) => {}
        }
    }
}

/// Develops a raw file, and tone maps it at full size and at each halving down to
/// [`SMALLEST_LEVEL`]. Halving happens before tone mapping, so it averages light rather than
/// display values.
fn develop_levels(path: &Path, max_texture_side: usize) -> anyhow::Result<Developed> {
    // Until the app shows any node, it shows what the view transform is given, halved before the
    // view transform is applied.
    let graph = default_graph();
    let (view_transform, upstream) = graph
        .nodes()
        .find_map(|(_, node)| match &node.node {
            Node::ViewTransform(view_transform) => Some((view_transform, node.node.inputs())),
            _ => None,
        })
        .context("The default graph has no view transform")?;
    let upstream = upstream[0].wire.clone().context("The view transform isn't wired")?;
    let inputs = Inputs::new().bind("image", Input::Path(path.to_owned()));

    let start = Instant::now();
    let mut image = evaluate(&graph, &inputs, &upstream)?
        .as_scene_referred_image()?
        .clone();
    tracing::info!(elapsed = ?start.elapsed(), "Developed");
    let full_size = Vec2::new(image.width as f32, image.height as f32);

    let start = Instant::now();
    let mut levels = Vec::new();
    loop {
        let longest = image.width.max(image.height);
        if longest <= max_texture_side {
            let display = view_transform.evaluate(&image)?;
            levels.push(xpro_core::view::encode_srgb8(&display)?);
        }
        if longest <= SMALLEST_LEVEL {
            break;
        }
        image = Arc::new(xpro_core::geometry::halve(&image));
    }
    tracing::info!(elapsed = ?start.elapsed(), levels = levels.len(), "Applied view transform");

    Ok(Developed { levels, full_size })
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
    let widths: Vec<usize> = loaded.levels.iter().map(|level| level.size()[0]).collect();
    let level = &loaded.levels[sharpest_needed(&widths, image_rect.width() * ppp)];
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

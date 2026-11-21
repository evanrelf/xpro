//! The interactive app.

mod viewport;

use anyhow::anyhow;
use egui::{Color32, Rect, Sense, TextureHandle, TextureOptions, Vec2, pos2};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Instant,
};
use viewport::{Viewer, Viewport};
use xpro_core::image::DisplayImage;

pub fn run(photo: Option<PathBuf>) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // On macOS, eframe also uses the initial title as the name in the
            // menu bar, so the window is renamed after the photo below.
            .with_title("Xpro")
            .with_min_inner_size([480.0, 320.0])
            .with_maximized(true),
        ..Default::default()
    };
    eframe::run_native(
        "Xpro",
        options,
        Box::new(|cc| {
            if let Some(path) = &photo {
                cc.egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Title(file_name(path)));
            }
            Ok(Box::new(App {
                photo: photo.map_or(Photo::None, Photo::Pending),
            }))
        }),
    )
    .map_err(|error| anyhow!("{error}"))
}

struct App {
    photo: Photo,
}

enum Photo {
    None,
    /// Waiting for the first frame, when the GPU's texture size limit is known.
    Pending(PathBuf),
    Loading(mpsc::Receiver<anyhow::Result<Developed>>),
    Loaded(Loaded),
    Failed(anyhow::Error),
}

/// A developed photo at several sizes, so it can be drawn sharply at any zoom.
struct Developed {
    /// The photo at full size, then halved again and again down to about screen size. Levels
    /// too big for the GPU are left out.
    levels: Vec<DisplayImage>,
    /// The size of the full photo, in pixels.
    full_size: Vec2,
}

struct Loaded {
    /// Textures of [`Developed::levels`], largest first.
    levels: Vec<TextureHandle>,
    /// The size of the full photo, in pixels.
    full_size: Vec2,
    viewport: Viewport,
}

/// Levels stop being halved once they're this small on their longest side, which is about as
/// small as the window can show the photo.
const SMALLEST_LEVEL: usize = 512;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.update_photo(ui.ctx());

        egui::CentralPanel::default().show(ui, |ui| match &mut self.photo {
            Photo::None => {
                ui.centered_and_justified(|ui| ui.label("Open a raw photo with `xpro PHOTO`"));
            }
            Photo::Pending(_) | Photo::Loading(_) => {
                ui.centered_and_justified(egui::Ui::spinner);
            }
            Photo::Loaded(loaded) => show_photo(ui, loaded),
            Photo::Failed(error) => {
                ui.centered_and_justified(|ui| ui.label(format!("{error:#}")));
            }
        });
    }
}

impl App {
    fn update_photo(&mut self, ctx: &egui::Context) {
        match &self.photo {
            Photo::Pending(path) => {
                let max_texture_side = ctx.input(|input| input.max_texture_side);
                let (sender, receiver) = mpsc::channel();
                let path = path.clone();
                let ctx = ctx.clone();
                thread::spawn(move || {
                    // If the app has quit, nobody needs the result.
                    let _ = sender.send(develop_levels(&path, max_texture_side));
                    ctx.request_repaint();
                });
                self.photo = Photo::Loading(receiver);
            }
            Photo::Loading(receiver) => match receiver.try_recv() {
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
                            ctx.load_texture(format!("photo {i}"), image, TextureOptions::LINEAR)
                        })
                        .collect();
                    tracing::info!(elapsed = ?start.elapsed(), "Uploaded textures");
                    self.photo = Photo::Loaded(Loaded {
                        levels,
                        full_size,
                        viewport: Viewport::Fit,
                    });
                }
                Ok(Err(error)) => self.photo = Photo::Failed(error),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.photo = Photo::Failed(anyhow!("Developing the photo crashed"));
                }
            },
            Photo::None | Photo::Loaded(_) | Photo::Failed(_) => {}
        }
    }
}

/// Develops a photo, and tone maps it at full size and at each halving down to
/// [`SMALLEST_LEVEL`]. Halving happens before tone mapping, so it averages light rather than
/// display values.
fn develop_levels(path: &Path, max_texture_side: usize) -> anyhow::Result<Developed> {
    let start = Instant::now();
    let raw = xpro_core::raw::decode(path)?;
    tracing::info!(elapsed = ?start.elapsed(), "Decoded");

    let start = Instant::now();
    let mut image = xpro_core::develop::develop(&raw)?;
    tracing::info!(elapsed = ?start.elapsed(), "Developed");
    let full_size = Vec2::new(image.width as f32, image.height as f32);

    let start = Instant::now();
    let curve = xpro_core::view::ToneCurve::standard();
    let mut levels = Vec::new();
    loop {
        let longest = image.width.max(image.height);
        if longest <= max_texture_side {
            levels.push(xpro_core::view::tone_map_to_srgb(&image, &curve));
        }
        if longest <= SMALLEST_LEVEL {
            break;
        }
        image = xpro_core::geometry::halve(&image);
    }
    tracing::info!(elapsed = ?start.elapsed(), levels = levels.len(), "Applied view transform");

    Ok(Developed { levels, full_size })
}

/// Shows the photo. Pinching zooms, swiping with two fingers pans, and double-clicking switches
/// between fitting and 100%.
fn show_photo(ui: &mut egui::Ui, loaded: &mut Loaded) {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click());
    let viewer = Viewer {
        rect,
        pixels_per_point: ui.pixels_per_point(),
        photo: loaded.full_size,
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

    // Line the photo up with the screen's pixels, so that at 100% each photo pixel lands on
    // exactly one screen pixel instead of blurring across two.
    let ppp = viewer.pixels_per_point;
    let photo_rect = viewer.photo_rect(viewport);
    let photo_rect = Rect::from_min_size(
        ((photo_rect.min.to_vec2() * ppp).round() / ppp).to_pos2(),
        photo_rect.size(),
    );
    let widths: Vec<usize> = loaded.levels.iter().map(|level| level.size()[0]).collect();
    let level = &loaded.levels[sharpest_needed(&widths, photo_rect.width() * ppp)];
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter_at(rect)
        .image(level.id(), photo_rect, uv, Color32::WHITE);
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

    /// Widths of a full-size photo and its successive halvings.
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

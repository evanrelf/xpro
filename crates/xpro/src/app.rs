//! The interactive app.

use anyhow::anyhow;
use egui::{Color32, Rect, TextureHandle, TextureOptions, Vec2, pos2};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Instant,
};
use xpro_core::image::DisplayImage;

pub fn run(photo: Option<PathBuf>) -> anyhow::Result<()> {
    let title = match &photo {
        Some(path) => format!("{} — Xpro", file_name(path)),
        None => "Xpro".to_owned(),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Xpro",
        options,
        Box::new(|_cc| {
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
    /// Waiting for the first frame, when the monitor's size is known.
    Pending(PathBuf),
    Loading(mpsc::Receiver<anyhow::Result<DisplayImage>>),
    Loaded(TextureHandle),
    Failed(anyhow::Error),
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.update_photo(ui.ctx());

        egui::CentralPanel::default().show(ui, |ui| match &self.photo {
            Photo::None => {
                ui.centered_and_justified(|ui| ui.label("Open a raw photo with `xpro PHOTO`"));
            }
            Photo::Pending(_) | Photo::Loading(_) => {
                ui.centered_and_justified(egui::Ui::spinner);
            }
            Photo::Loaded(texture) => show_fitted(ui, texture),
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
                let (max_width, max_height) = preview_size(ctx);
                let (sender, receiver) = mpsc::channel();
                let path = path.clone();
                let ctx = ctx.clone();
                thread::spawn(move || {
                    // If the app has quit, nobody needs the result.
                    let _ = sender.send(develop_preview(&path, max_width, max_height));
                    ctx.request_repaint();
                });
                self.photo = Photo::Loading(receiver);
            }
            Photo::Loading(receiver) => match receiver.try_recv() {
                Ok(Ok(image)) => {
                    let image = egui::ColorImage::from_rgb(
                        [image.width, image.height],
                        image.pixels.as_flattened(),
                    );
                    let texture = ctx.load_texture("photo", image, TextureOptions::LINEAR);
                    self.photo = Photo::Loaded(texture);
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

/// The largest the photo can be shown, in pixels: the size of the monitor, so the window can
/// grow to fill it, but no bigger than the GPU allows.
fn preview_size(ctx: &egui::Context) -> (usize, usize) {
    ctx.input(|input| {
        let max_texture_side = input.max_texture_side;
        let monitor = input
            .viewport()
            .monitor_size
            .map(|size| size * input.viewport().native_pixels_per_point.unwrap_or(1.0));
        match monitor {
            Some(size) => (
                (size.x.ceil() as usize).min(max_texture_side),
                (size.y.ceil() as usize).min(max_texture_side),
            ),
            None => (max_texture_side, max_texture_side),
        }
    })
}

fn develop_preview(
    path: &Path,
    max_width: usize,
    max_height: usize,
) -> anyhow::Result<DisplayImage> {
    let start = Instant::now();
    let raw = xpro_core::raw::decode(path)?;
    tracing::info!(elapsed = ?start.elapsed(), "Decoded");

    let start = Instant::now();
    let image = xpro_core::develop::develop(&raw)?;
    tracing::info!(elapsed = ?start.elapsed(), "Developed");

    let start = Instant::now();
    let image = xpro_core::geometry::shrink_to_fit(image, max_width, max_height);
    tracing::info!(
        elapsed = ?start.elapsed(),
        "Shrank to {}×{}",
        image.width,
        image.height,
    );

    let start = Instant::now();
    let display = xpro_core::view::clip_to_srgb(&image);
    tracing::info!(elapsed = ?start.elapsed(), "Applied view transform");

    Ok(display)
}

/// Shows the photo as large as fits in the available space, centered.
fn show_fitted(ui: &egui::Ui, texture: &TextureHandle) {
    let rect = fit(ui.available_rect_before_wrap(), texture.size_vec2());
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
}

/// The largest rect with the same aspect ratio as `size` that fits in `available`, centered in
/// it. If `available` has no room, the rect is empty.
fn fit(available: Rect, size: Vec2) -> Rect {
    let scale = (available.width() / size.x)
        .min(available.height() / size.y)
        .max(0.0);
    Rect::from_center_size(available.center(), size * scale)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hegel::{TestCase, generators as gs};

    #[hegel::composite]
    fn gen_length(tc: &TestCase, max: f32) -> f32 {
        tc.draw(gs::floats::<f32>().min_value(1.0).max_value(max))
    }

    #[hegel::composite]
    fn gen_fit_inputs(tc: &TestCase) -> (Rect, Vec2) {
        let min = pos2(
            tc.draw(gs::floats::<f32>().min_value(-1e4).max_value(1e4)),
            tc.draw(gs::floats::<f32>().min_value(-1e4).max_value(1e4)),
        );
        let available = Rect::from_min_size(
            min,
            Vec2::new(tc.draw(gen_length(1e4)), tc.draw(gen_length(1e4))),
        );
        let size = Vec2::new(tc.draw(gen_length(1e5)), tc.draw(gen_length(1e5)));
        (available, size)
    }

    /// How far off a computed coordinate inside `rect` can be from rounding: a few f32 steps at
    /// the rect's largest coordinate.
    fn position_error(rect: Rect) -> f32 {
        let magnitude = [rect.min.x, rect.min.y, rect.max.x, rect.max.y]
            .into_iter()
            .fold(1.0, |max: f32, x| max.max(x.abs()));
        8.0 * f32::EPSILON * magnitude
    }

    fn close(a: f32, b: f32, absolute: f32) -> bool {
        (a - b).abs() <= absolute + 1e-5 * a.abs().max(b.abs())
    }

    #[hegel::test]
    fn fit_is_never_inverted(tc: TestCase) {
        // A cramped layout can leave a negative amount of space. Then there's no room for the
        // photo, and it mustn't be drawn mirrored instead.
        let side = || gs::floats::<f32>().min_value(-1e4).max_value(1e4);
        let min = pos2(tc.draw(side()), tc.draw(side()));
        let available = Rect::from_min_size(min, Vec2::new(tc.draw(side()), tc.draw(side())));
        let size = Vec2::new(tc.draw(gen_length(1e5)), tc.draw(gen_length(1e5)));
        let rect = fit(available, size);
        assert!(rect.width() >= 0.0 && rect.height() >= 0.0, "{rect:?}");
    }

    #[hegel::test]
    fn fit_stays_inside_the_available_space(tc: TestCase) {
        let (available, size) = tc.draw(gen_fit_inputs());
        let rect = fit(available, size);
        let error = position_error(available);
        assert!(available.expand(error).contains_rect(rect), "{rect:?} in {available:?}");
    }

    #[hegel::test]
    fn fit_keeps_the_aspect_ratio(tc: TestCase) {
        let (available, size) = tc.draw(gen_fit_inputs());
        let rect = fit(available, size);
        // Compare cross products rather than ratios, to avoid dividing by tiny heights.
        let error = position_error(available);
        assert!(
            close(
                rect.width() * size.y,
                rect.height() * size.x,
                error * (size.x + size.y),
            ),
            "{rect:?} vs {size:?}",
        );
    }

    #[hegel::test]
    fn fit_is_centered(tc: TestCase) {
        let (available, size) = tc.draw(gen_fit_inputs());
        let rect = fit(available, size);
        let error = position_error(available);
        assert!(close(rect.center().x, available.center().x, error));
        assert!(close(rect.center().y, available.center().y, error));
    }

    #[hegel::test]
    fn fit_fills_one_dimension(tc: TestCase) {
        // Otherwise it could be bigger.
        let (available, size) = tc.draw(gen_fit_inputs());
        let rect = fit(available, size);
        let error = position_error(available);
        assert!(
            close(rect.width(), available.width(), error)
                || close(rect.height(), available.height(), error),
            "{rect:?} in {available:?}",
        );
    }
}

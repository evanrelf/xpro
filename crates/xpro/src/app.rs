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
    let title = match &photo {
        Some(path) => format!("{} — Xpro", file_name(path)),
        None => "Xpro".to_owned(),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_min_inner_size([480.0, 320.0])
            .with_maximized(true),
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
    Loading(mpsc::Receiver<anyhow::Result<Preview>>),
    Loaded(Loaded),
    Failed(anyhow::Error),
}

/// A developed photo, shrunk to fit the screen.
struct Preview {
    image: DisplayImage,
    /// The size of the full photo, in pixels.
    full_size: Vec2,
}

struct Loaded {
    texture: TextureHandle,
    /// The size of the full photo, in pixels. The texture may be smaller.
    full_size: Vec2,
    viewport: Viewport,
}

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
                Ok(Ok(Preview { image, full_size })) => {
                    let image = egui::ColorImage::from_rgb(
                        [image.width, image.height],
                        image.pixels.as_flattened(),
                    );
                    let texture = ctx.load_texture("photo", image, TextureOptions::LINEAR);
                    self.photo = Photo::Loaded(Loaded {
                        texture,
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
) -> anyhow::Result<Preview> {
    let start = Instant::now();
    let raw = xpro_core::raw::decode(path)?;
    tracing::info!(elapsed = ?start.elapsed(), "Decoded");

    let start = Instant::now();
    let image = xpro_core::develop::develop(&raw)?;
    tracing::info!(elapsed = ?start.elapsed(), "Developed");
    let full_size = Vec2::new(image.width as f32, image.height as f32);

    let start = Instant::now();
    let image = xpro_core::geometry::shrink_to_fit(image, max_width, max_height);
    tracing::info!(
        elapsed = ?start.elapsed(),
        "Shrank to {}×{}",
        image.width,
        image.height,
    );

    let start = Instant::now();
    let display = xpro_core::view::tone_map_to_srgb(&image, &xpro_core::view::ToneCurve::standard());
    tracing::info!(elapsed = ?start.elapsed(), "Applied view transform");

    Ok(Preview {
        image: display,
        full_size,
    })
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

    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter_at(rect).image(
        loaded.texture.id(),
        viewer.photo_rect(viewport),
        uv,
        Color32::WHITE,
    );
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod ui_tests;

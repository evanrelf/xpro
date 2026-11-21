//! Headless tests of the whole app, driven and rendered with `egui_kittest`.

use super::*;
use egui_kittest::{Harness, kittest::Queryable as _};
use std::time::Duration;

const WINDOW: Vec2 = Vec2::new(400.0, 300.0);

fn harness(photo: Photo) -> Harness<'static, App> {
    Harness::builder()
        .with_size(WINDOW)
        .with_pixels_per_point(1.0)
        .build_eframe(|_cc| App { photo })
}

/// A photo whose development has already finished with the given result.
fn developed(result: anyhow::Result<DisplayImage>) -> Photo {
    let (sender, receiver) = mpsc::channel();
    sender.send(result).unwrap();
    Photo::Loading(receiver)
}

/// A `width`×`height` image whose left half is red and right half is blue.
fn red_and_blue(width: usize, height: usize) -> DisplayImage {
    let pixels = (0..height)
        .flat_map(|_| (0..width).map(|x| if x < width / 2 { [255, 0, 0] } else { [0, 0, 255] }))
        .collect();
    DisplayImage {
        width,
        height,
        pixels,
    }
}

#[test]
fn without_a_photo_it_explains_how_to_open_one() {
    let mut harness = harness(Photo::None);
    harness.run();
    harness.get_by_label_contains("xpro PHOTO");
}

#[test]
fn a_failure_shows_the_error() {
    let mut harness = harness(developed(Err(anyhow!("Not a raw file"))));
    harness.run();
    harness.get_by_label_contains("Not a raw file");
}

#[test]
fn a_crash_while_developing_is_reported() {
    let (sender, receiver) = mpsc::channel();
    drop(sender);
    let mut harness = harness(Photo::Loading(receiver));
    harness.run();
    harness.get_by_label_contains("crashed");
}

#[test]
fn a_photo_is_shown_fitted_and_centered() {
    // Wider than the window, so it fills the width and leaves bands above and below.
    let mut harness = harness(developed(Ok(red_and_blue(200, 100))));
    harness.run();
    assert!(matches!(harness.state().photo, Photo::Loaded(_)));

    let render = harness.render().unwrap();
    let pixel = |x: f32, y: f32| {
        let [r, g, b, _] = render.get_pixel(x as u32, y as u32).0;
        [r, g, b]
    };
    let background = pixel(1.0, 1.0);
    let (center_x, center_y) = (WINDOW.x / 2.0, WINDOW.y / 2.0);

    // Not mirrored: red on the left, blue on the right, out to near the window's edges.
    assert_eq!(pixel(WINDOW.x * 0.1, center_y), [255, 0, 0]);
    assert_eq!(pixel(center_x - 5.0, center_y), [255, 0, 0]);
    assert_eq!(pixel(center_x + 5.0, center_y), [0, 0, 255]);
    assert_eq!(pixel(WINDOW.x * 0.9, center_y), [0, 0, 255]);
    // At 2:1 in a 4:3 window, the photo covers the middle and leaves bands above and below.
    assert_eq!(pixel(center_x, WINDOW.y * 0.1), background);
    assert_eq!(pixel(center_x, WINDOW.y * 0.9), background);
    assert_eq!(pixel(center_x - 5.0, WINDOW.y * 0.25), [255, 0, 0]);
    assert_eq!(pixel(center_x + 5.0, WINDOW.y * 0.75), [0, 0, 255]);
}

/// Develops a real raw photo end to end. Set `XPRO_SAMPLE_PHOTO` to a raw file to run it, and
/// `XPRO_TEST_RENDERS` to a directory to save what the window shows.
#[test]
fn a_sample_photo_develops_and_is_shown() {
    let Some(path) = std::env::var_os("XPRO_SAMPLE_PHOTO").map(PathBuf::from) else {
        eprintln!("Skipping: set XPRO_SAMPLE_PHOTO to a raw file to run this test");
        return;
    };
    let mut harness = harness(Photo::Pending(path.clone()));
    let deadline = Instant::now() + Duration::from_secs(300);
    while matches!(harness.state().photo, Photo::Pending(_) | Photo::Loading(_)) {
        assert!(Instant::now() < deadline, "Timed out developing {}", path.display());
        harness.step();
        thread::sleep(Duration::from_millis(50));
    }
    if let Photo::Failed(error) = &harness.state().photo {
        panic!("{error:#}");
    }
    harness.run();

    let render = harness.render().unwrap();
    if let Some(dir) = std::env::var_os("XPRO_TEST_RENDERS").map(PathBuf::from) {
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("{}.png", file_name(&path)));
        render.save(&file).unwrap();
        eprintln!("Saved {}", file.display());
    }
}

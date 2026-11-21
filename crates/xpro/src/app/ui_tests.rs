//! Headless tests of the whole app, driven and rendered with `egui_kittest`.

use super::*;
use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, PointerButton, Pos2, TouchPhase};
use egui_kittest::{Harness, kittest::Queryable as _};
use std::time::Duration;

const WINDOW: Vec2 = Vec2::new(400.0, 300.0);

fn harness(photo: Photo) -> Harness<'static, App> {
    Harness::builder()
        .with_size(WINDOW)
        .with_pixels_per_point(1.0)
        // Short frames, so two clicks are quick enough to count as a double-click.
        .with_step_dt(1.0 / 60.0)
        // Smooth scrolling takes a few frames to settle.
        .with_max_steps(60)
        .build_eframe(|_cc| App { photo })
}

/// A photo whose development has already finished with the given result.
fn developed(result: anyhow::Result<DisplayImage>) -> Photo {
    let (sender, receiver) = mpsc::channel();
    let preview = result.map(|image| Preview {
        full_size: Vec2::new(image.width as f32, image.height as f32),
        image,
    });
    sender.send(preview).unwrap();
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

/// A loaded photo much bigger than the window, so it can be zoomed in.
fn big_photo() -> Harness<'static, App> {
    let mut harness = harness(developed(Ok(red_and_blue(1600, 800))));
    harness.run();
    harness
}

fn viewport(harness: &Harness<'_, App>) -> Viewport {
    match &harness.state().photo {
        Photo::Loaded(loaded) => loaded.viewport,
        _ => panic!("No photo loaded"),
    }
}

fn zoomed(harness: &Harness<'_, App>) -> (f32, Pos2) {
    match viewport(harness) {
        Viewport::Zoomed { scale, center } => (scale, center),
        Viewport::Fit => panic!("Not zoomed"),
    }
}

fn pinch(harness: &mut Harness<'_, App>, at: Pos2, factor: f32) {
    harness.hover_at(at);
    harness.event(Event::Zoom(factor));
    harness.run();
}

fn swipe(harness: &mut Harness<'_, App>, delta: Vec2) {
    harness.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta,
        phase: TouchPhase::Move,
        source: MouseWheelSource::Trackpad,
        modifiers: Modifiers::NONE,
    });
    harness.run();
}

fn double_click(harness: &mut Harness<'_, App>, at: Pos2) {
    // Pause first, so this doesn't continue an earlier burst of clicks as a triple-click.
    harness.run_steps(30);
    harness.hover_at(at);
    for _ in 0..2 {
        for pressed in [true, false] {
            harness.event(Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
    }
    harness.run();
}

const CENTER: Pos2 = Pos2::new(WINDOW.x / 2.0, WINDOW.y / 2.0);

#[test]
fn a_photo_starts_out_fitted() {
    assert_eq!(viewport(&big_photo()), Viewport::Fit);
}

#[test]
fn spreading_fingers_zooms_in_around_them() {
    let mut harness = big_photo();
    pinch(&mut harness, CENTER, 2.0);
    let (scale, center) = zoomed(&harness);
    // In this window, the photo fits at about 0.24 points per pixel.
    assert!((0.4..0.6).contains(&scale), "{scale}");
    assert!((center.x - 800.0).abs() < 1.0 && (center.y - 400.0).abs() < 1.0, "{center:?}");

    // Zooming in on the left keeps the left side in view.
    pinch(&mut harness, Pos2::new(WINDOW.x * 0.1, WINDOW.y / 2.0), 2.0);
    let (_, center) = zoomed(&harness);
    assert!(center.x < 800.0, "{center:?}");
}

#[test]
fn pinching_back_out_fits_again() {
    let mut harness = big_photo();
    pinch(&mut harness, CENTER, 4.0);
    pinch(&mut harness, CENTER, 0.1);
    assert_eq!(viewport(&harness), Viewport::Fit);
}

#[test]
fn swiping_pans_a_zoomed_photo() {
    let mut harness = big_photo();
    pinch(&mut harness, CENTER, 4.0);
    let (scale, before) = zoomed(&harness);
    // Swiping right moves the photo right, revealing more of its left side.
    swipe(&mut harness, Vec2::new(50.0, 0.0));
    let (_, after) = zoomed(&harness);
    assert!((before.x - after.x - 50.0 / scale).abs() < 1.0, "{before:?} → {after:?}");
    assert!((before.y - after.y).abs() < 1.0, "{before:?} → {after:?}");
}

#[test]
fn swiping_leaves_a_fitted_photo_in_place() {
    let mut harness = big_photo();
    swipe(&mut harness, Vec2::new(50.0, 30.0));
    assert_eq!(viewport(&harness), Viewport::Fit);
}

#[test]
fn double_clicking_switches_between_fit_and_100_percent() {
    let mut harness = big_photo();
    double_click(&mut harness, CENTER);
    let (scale, _) = zoomed(&harness);
    assert!((scale - 1.0).abs() < 1e-4, "{scale}");

    double_click(&mut harness, CENTER);
    assert_eq!(viewport(&harness), Viewport::Fit);
}

#[test]
fn a_zoomed_photo_is_drawn_bigger() {
    // At 100%, the 1600-pixel-wide photo is far wider than the window, so it's red on the left
    // edge of the window (zoomed in on the red half) with no background showing.
    let mut harness = big_photo();
    double_click(&mut harness, Pos2::new(WINDOW.x * 0.25, WINDOW.y / 2.0));
    let render = harness.render().unwrap();
    for x in [10, 200, 390] {
        let [r, g, b, _] = render.get_pixel(x, 150).0;
        assert_eq!([r, g, b], [255, 0, 0], "at x = {x}");
    }
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

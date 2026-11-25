//! Headless tests of the whole app, driven and rendered with `egui_kittest`.

use super::*;
use egui::{Event, Modifiers, MouseWheelSource, MouseWheelUnit, PointerButton, Pos2, TouchPhase};
use egui_kittest::{Harness, kittest::Queryable as _};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use worker::SMALLEST_LEVEL;
use xpro_core::{
    color::REC2020,
    eval::{IMAGE_INPUT, Input},
    geometry::Orientation,
    node::{Node, WritePng},
    raw::{Channel, Mosaic},
};

const WINDOW: Vec2 = Vec2::new(400.0, 300.0);

/// The viewer on its own, filling the window.
struct ViewerState {
    loaded: Loaded,
}

impl eframe::App for ViewerState {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| show_image(ui, &mut self.loaded));
    }
}

fn viewer(image: Srgb8Image) -> Harness<'static, ViewerState> {
    Harness::builder()
        .with_size(WINDOW)
        .with_pixels_per_point(1.0)
        // Short frames, so two clicks are quick enough to count as a double-click.
        .with_step_dt(1.0 / 60.0)
        // Smooth scrolling takes a few frames to settle.
        .with_max_steps(60)
        .build_eframe(|cc| {
            let full_size = Vec2::new(image.width as f32, image.height as f32);
            let levels = (0..)
                .zip(&levels(image))
                .map(|(halvings, level)| (halvings, upload(&cc.egui_ctx, level, 1, halvings)))
                .collect();
            ViewerState {
                loaded: Loaded {
                    levels,
                    full_size,
                    viewport: Viewport::Fit,
                    generation: 1,
                },
            }
        })
}

/// The image followed by successive halvings, like the worker makes. Each halving just keeps
/// every other pixel, which is all these tests need.
fn levels(image: Srgb8Image) -> Vec<Srgb8Image> {
    let mut levels = vec![image];
    loop {
        let last = levels.last().unwrap();
        if last.width.max(last.height) <= SMALLEST_LEVEL {
            return levels;
        }
        let (width, height) = (last.width.div_ceil(2), last.height.div_ceil(2));
        let pixels = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| last.pixels[2 * y * last.width + 2 * x])
            .collect();
        levels.push(Srgb8Image {
            width,
            height,
            pixels,
        });
    }
}

/// A `width`×`height` image whose left half is red and right half is blue.
fn red_and_blue(width: usize, height: usize) -> Srgb8Image {
    let pixels = (0..height)
        .flat_map(|_| (0..width).map(|x| if x < width / 2 { [255, 0, 0] } else { [0, 0, 255] }))
        .collect();
    Srgb8Image {
        width,
        height,
        pixels,
    }
}

/// The whole app, in a window big enough for its panels and a viewer.
fn app(document: Option<anyhow::Result<Document>>) -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1000.0, 600.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| App::new(&cc.egui_ctx, document));
    // The harness reports egui's fallback texture limit, 2048. Report what the app's GPU setup
    // actually allows instead.
    harness.input_mut().max_texture_side = Some(8192);
    harness
}

/// Steps the app until its worker has finished everything it was given.
fn settle(harness: &mut Harness<'_, App>) {
    let deadline = Instant::now() + Duration::from_secs(300);
    harness.step();
    while !harness.state().is_idle() {
        assert!(Instant::now() < deadline, "Timed out evaluating");
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.run();
}

/// A document editing an image file in a temporary directory with the default graph, whose
/// graph input is bound to a small mosaic of a gray gradient instead of the image file. The
/// output writes `out.png`, since the mosaic has no file name to name it after.
fn document(dir: &tempfile::TempDir) -> Document {
    let (width, height) = (64, 48);
    let mosaic = Mosaic {
        width,
        height,
        cfa: [[Channel::Red, Channel::Green], [Channel::Green, Channel::Blue]],
        values: (0..width * height)
            .map(|i| (i % width) as f32 / width as f32 * 0.25)
            .collect(),
        as_shot_white_balance: [1.0, 1.0, 1.0],
        xyz_to_camera: REC2020.rgb_to_xyz().inverse().unwrap(),
        orientation: Orientation::Normal,
    };
    let mut graph = default_graph();
    for (_, node) in graph.nodes_mut() {
        if let Node::WritePng(output) = &mut node.node {
            *output = WritePng {
                path: "out.png".to_owned(),
                ..output.clone()
            };
        }
    }
    let inputs = Inputs::new().bind(IMAGE_INPUT, Input::Mosaic(Arc::new(mosaic)));
    Document::new(dir.path().join("DSC_4362.NEF"), Edit::new(graph), inputs)
}

fn document_state<'a>(harness: &'a Harness<'_, App>) -> &'a Document {
    match &harness.state().document {
        Some(Ok(document)) => document,
        _ => panic!("No document"),
    }
}

/// The node list's entry for a node, which is on the left.
fn node_in_list<'a>(
    harness: &'a Harness<'_, App>,
    name: &'a str,
) -> egui_kittest::Node<'a> {
    harness
        .get_all_by_label(name)
        .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
        .unwrap_or_else(|| panic!("No {name} in the node list"))
}

/// The average brightness of the middle of the viewer, from 0 to 255.
fn shown_brightness(harness: &mut Harness<'_, App>) -> f32 {
    harness.remove_cursor();
    settle(harness);
    let render = harness.render().unwrap();
    let mut sum = 0.0;
    let mut count = 0.0;
    for y in 250..350 {
        for x in 420..580 {
            let [r, g, b, _] = render.get_pixel(x, y).0;
            sum += f32::from(r) + f32::from(g) + f32::from(b);
            count += 3.0;
        }
    }
    sum / count
}

/// A slider in the inspector.
fn slider<'a>(harness: &'a Harness<'_, App>, name: &'a str) -> egui_kittest::Node<'a> {
    harness.get_by_role_and_label(egui::accesskit::Role::Slider, name)
}

/// Sets a slider in the inspector, as if dragged there.
fn set_slider(harness: &mut Harness<'_, App>, name: &str, value: f64) {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Slider, name)
        .set_value_accesskit(value);
    settle(harness);
}

#[test]
fn without_an_image_file_it_explains_how_to_open_one() {
    let mut harness = app(None);
    harness.run();
    harness.get_by_label_contains("xpro IMAGE_FILE");
}

#[test]
fn a_file_that_isnt_an_image_shows_the_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "Not a raw file").unwrap();
    let mut harness = app(Some(Document::open(&path)));
    settle(&mut harness);
    harness.get_by_label_contains("Failed to decode");
}

#[test]
fn a_crash_while_evaluating_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(Some(Ok(document(&dir))));
    let (sender, receiver) = mpsc::channel();
    drop(sender);
    harness.state_mut().worker = Some(Worker {
        jobs: None,
        updates: receiver,
        cancel: xpro_core::request::Cancel::new(),
        thread: None,
    });
    settle(&mut harness);
    harness.get_by_label_contains("crashed");
}

#[test]
fn an_image_is_shown_fitted_and_centered() {
    // Wider than the window, so it fills the width and leaves bands above and below.
    let mut harness = viewer(red_and_blue(200, 100));
    harness.run();

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
    // At 2:1 in a 4:3 window, the image covers the middle and leaves bands above and below.
    assert_eq!(pixel(center_x, WINDOW.y * 0.1), background);
    assert_eq!(pixel(center_x, WINDOW.y * 0.9), background);
    assert_eq!(pixel(center_x - 5.0, WINDOW.y * 0.25), [255, 0, 0]);
    assert_eq!(pixel(center_x + 5.0, WINDOW.y * 0.75), [0, 0, 255]);
}

/// A loaded image much bigger than the window, so it can be zoomed in.
fn big_image() -> Harness<'static, ViewerState> {
    let mut harness = viewer(red_and_blue(1600, 800));
    harness.run();
    harness
}

fn viewport(harness: &Harness<'_, ViewerState>) -> Viewport {
    harness.state().loaded.viewport
}

fn zoomed(harness: &Harness<'_, ViewerState>) -> (f32, Pos2) {
    match viewport(harness) {
        Viewport::Zoomed { scale, center } => (scale, center),
        Viewport::Fit => panic!("Not zoomed"),
    }
}

fn pinch(harness: &mut Harness<'_, ViewerState>, at: Pos2, factor: f32) {
    harness.hover_at(at);
    harness.event(Event::Zoom(factor));
    harness.run();
}

fn swipe(harness: &mut Harness<'_, ViewerState>, delta: Vec2) {
    harness.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta,
        phase: TouchPhase::Move,
        source: MouseWheelSource::Trackpad,
        modifiers: Modifiers::NONE,
    });
    harness.run();
}

fn double_click(harness: &mut Harness<'_, ViewerState>, at: Pos2) {
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
fn an_image_starts_out_fitted() {
    assert_eq!(viewport(&big_image()), Viewport::Fit);
}

#[test]
fn spreading_fingers_zooms_in_around_them() {
    let mut harness = big_image();
    pinch(&mut harness, CENTER, 2.0);
    let (scale, center) = zoomed(&harness);
    // In this window, the image fits at about 0.24 points per pixel.
    assert!((0.4..0.6).contains(&scale), "{scale}");
    assert!((center.x - 800.0).abs() < 1.0 && (center.y - 400.0).abs() < 1.0, "{center:?}");

    // Zooming in on the left keeps the left side in view.
    pinch(&mut harness, Pos2::new(WINDOW.x * 0.1, WINDOW.y / 2.0), 2.0);
    let (_, center) = zoomed(&harness);
    assert!(center.x < 800.0, "{center:?}");
}

#[test]
fn pinching_back_out_fits_again() {
    let mut harness = big_image();
    pinch(&mut harness, CENTER, 4.0);
    pinch(&mut harness, CENTER, 0.1);
    assert_eq!(viewport(&harness), Viewport::Fit);
}

#[test]
fn swiping_pans_a_zoomed_image() {
    let mut harness = big_image();
    pinch(&mut harness, CENTER, 4.0);
    let (scale, before) = zoomed(&harness);
    // Swiping right moves the image right, revealing more of its left side.
    swipe(&mut harness, Vec2::new(50.0, 0.0));
    let (_, after) = zoomed(&harness);
    assert!((before.x - after.x - 50.0 / scale).abs() < 1.0, "{before:?} → {after:?}");
    assert!((before.y - after.y).abs() < 1.0, "{before:?} → {after:?}");
}

#[test]
fn swiping_leaves_a_fitted_image_in_place() {
    let mut harness = big_image();
    swipe(&mut harness, Vec2::new(50.0, 30.0));
    assert_eq!(viewport(&harness), Viewport::Fit);
}

#[test]
fn double_clicking_switches_between_fit_and_100_percent() {
    let mut harness = big_image();
    double_click(&mut harness, CENTER);
    let (scale, _) = zoomed(&harness);
    assert!((scale - 1.0).abs() < 1e-4, "{scale}");

    double_click(&mut harness, CENTER);
    assert_eq!(viewport(&harness), Viewport::Fit);
}

#[test]
fn at_100_percent_each_image_pixel_is_one_screen_pixel() {
    // A checkerboard of single pixels only survives being drawn if it isn't shrunk first and
    // lines up exactly with the screen's pixels.
    let (width, height) = (1600, 800);
    let pixels = (0..height)
        .flat_map(|y| (0..width).map(move |x| if (x + y) % 2 == 0 { [255; 3] } else { [0; 3] }))
        .collect();
    let mut harness = viewer(Srgb8Image {
        width,
        height,
        pixels,
    });
    harness.run();
    // Off-center, between screen pixels, so the image would land between them too.
    double_click(&mut harness, Pos2::new(203.3, 151.6));
    // Keep the mouse cursor out of the render.
    harness.remove_cursor();
    harness.run();
    let render = harness.render().unwrap();
    for y in 100..200 {
        for x in 100..300 {
            let [here, ..] = render.get_pixel(x, y).0;
            let [right, ..] = render.get_pixel(x + 1, y).0;
            assert!(here.abs_diff(right) > 250, "blurred at ({x}, {y}): {here} next to {right}");
        }
    }
}

#[test]
fn a_zoomed_image_is_drawn_bigger() {
    // At 100%, the 1600-pixel-wide image is far wider than the window, so it's red on the left
    // edge of the window (zoomed in on the red half) with no background showing.
    let mut harness = big_image();
    double_click(&mut harness, Pos2::new(WINDOW.x * 0.25, WINDOW.y / 2.0));
    let render = harness.render().unwrap();
    for x in [10, 200, 390] {
        let [r, g, b, _] = render.get_pixel(x, 150).0;
        assert_eq!([r, g, b], [255, 0, 0], "at x = {x}");
    }
}

#[test]
fn the_node_list_shows_nodes_in_the_order_values_flow() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(Some(Ok(document(&dir))));
    settle(&mut harness);
    let names = [
        "Read Raw",
        "White Balance",
        "Demosaic",
        "Camera to Working Space",
        "Orient",
        "Exposure",
        "View Transform",
        "Write PNG",
    ];
    let tops: Vec<f32> = names
        .iter()
        .map(|name| node_in_list(&harness, name).rect().top())
        .collect();
    assert!(tops.is_sorted(), "{tops:?}");
}

#[test]
fn the_viewer_starts_on_what_the_output_writes() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(Some(Ok(document(&dir))));
    settle(&mut harness);
    let selected = document_state(&harness).selected.clone().unwrap();
    let graph = &document_state(&harness).edit.graph;
    assert!(matches!(graph.get(&selected).unwrap().node, Node::ViewTransform(_)));
    slider(&harness, "Contrast");
    assert!(shown_brightness(&mut harness) > 10.0);
}

#[test]
fn clicking_a_node_selects_it_and_shows_its_parameters() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(Some(Ok(document(&dir))));
    settle(&mut harness);
    assert!(
        harness
            .query_by_role_and_label(egui::accesskit::Role::Slider, "Stops")
            .is_none()
    );
    node_in_list(&harness, "Exposure").click();
    settle(&mut harness);
    slider(&harness, "Stops");
    let selected = document_state(&harness).selected.clone().unwrap();
    let graph = &document_state(&harness).edit.graph;
    assert!(matches!(graph.get(&selected).unwrap().node, Node::Exposure(_)));
}

#[test]
fn raising_exposure_brightens_the_image() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(Some(Ok(document(&dir))));
    settle(&mut harness);
    let before = shown_brightness(&mut harness);
    node_in_list(&harness, "Exposure").click();
    settle(&mut harness);
    set_slider(&mut harness, "Stops", 3.0);
    // The viewer shows Exposure now, through the default view transform, which matches the
    // graph's.
    let after = shown_brightness(&mut harness);
    assert!(after > before + 10.0, "{before} → {after}");
    assert!(document_state(&harness).is_edited());
    harness.get_by_label("Edited");
}

#[test]
fn an_invalid_parameter_shows_the_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(Some(Ok(document(&dir))));
    settle(&mut harness);
    set_slider(&mut harness, "Contrast", 0.1);
    harness.get_by_label_contains("can't reach white");
}

#[test]
fn saving_writes_the_edit_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(Some(Ok(document(&dir))));
    settle(&mut harness);
    node_in_list(&harness, "Exposure").click();
    settle(&mut harness);
    set_slider(&mut harness, "Stops", 2.0);
    harness.key_press_modifiers(Modifiers::COMMAND, egui::Key::S);
    settle(&mut harness);
    let document = document_state(&harness);
    assert!(!document.is_edited());
    let saved = Edit::read(&Edit::path_for(&document.image_path)).unwrap();
    assert_eq!(saved, document.edit);
    assert!(harness.query_by_label("Edited").is_none());
}

#[test]
fn exporting_writes_what_the_output_nodes_describe() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = app(Some(Ok(document(&dir))));
    settle(&mut harness);
    harness.get_by_label("Export").click();
    settle(&mut harness);
    harness.get_by_label_contains("Exported out.png");
    let written = ::image::open(dir.path().join("out.png")).unwrap();
    assert_eq!((written.width(), written.height()), (64, 48));

    // Exporting again would overwrite it, so it fails.
    harness.get_by_label("Export").click();
    settle(&mut harness);
    harness.get_by_label_contains("already exists");
}

/// Develops a real raw file end to end. Set `XPRO_SAMPLE_RAW_FILE` to a raw file to run it, and
/// `XPRO_TEST_RENDERS` to a directory to save what the window shows, fitted and at 100%.
#[test]
fn a_sample_raw_file_develops_and_is_shown() {
    let Some(path) = std::env::var_os("XPRO_SAMPLE_RAW_FILE").map(PathBuf::from) else {
        eprintln!("Skipping: set XPRO_SAMPLE_RAW_FILE to a raw file to run this test");
        return;
    };
    let mut harness = app(Some(Document::open(&path)));
    settle(&mut harness);
    if let Shown::Failed(error) = &harness.state().shown {
        panic!("{error}");
    }
    let Shown::Image(loaded) = &harness.state().shown else {
        panic!("Nothing shown");
    };
    assert_eq!(loaded.levels[0].0, 0, "the full-size level is shown");
    save_render(&mut harness, &format!("{}.png", file_name(&path)));
}

/// Saves what the window shows to `XPRO_TEST_RENDERS`, if it's set.
fn save_render<State>(harness: &mut Harness<'_, State>, name: &str) {
    let render = harness.render().unwrap();
    if let Some(dir) = std::env::var_os("XPRO_TEST_RENDERS").map(PathBuf::from) {
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(name);
        render.save(&file).unwrap();
        eprintln!("Saved {}", file.display());
    }
}

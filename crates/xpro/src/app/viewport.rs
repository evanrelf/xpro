//! Where the image sits in the viewer: fitting, zooming and panning.

use egui::{Pos2, Rect, Vec2};

/// The most the image can be magnified, in screen pixels per image pixel.
const MAX_MAGNIFICATION: f32 = 16.0;

/// How the image is placed in the viewer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Viewport {
    /// As large as fits, centered. Keeps fitting as the viewer changes size.
    #[default]
    Fit,
    /// Shown at `scale` points per image pixel, with `center` (a point on the image, in image
    /// pixels) at the middle of the viewer.
    Zoomed { scale: f32, center: Pos2 },
}

/// The space the image is shown in.
#[derive(Clone, Copy, Debug)]
pub struct Viewer {
    /// Where the viewer is, in points.
    pub rect: Rect,
    /// Screen pixels per point.
    pub pixels_per_point: f32,
    /// The image's size, in pixels.
    pub image: Vec2,
}

impl Viewer {
    /// The scale at which the image is as large as fits, in points per image pixel. Zero if the
    /// viewer has no room.
    pub fn fit_scale(&self) -> f32 {
        (self.rect.width() / self.image.x)
            .min(self.rect.height() / self.image.y)
            .max(0.0)
    }

    /// The scale at which one image pixel covers one screen pixel ("100%").
    pub fn actual_scale(&self) -> f32 {
        1.0 / self.pixels_per_point
    }

    fn max_scale(&self) -> f32 {
        (MAX_MAGNIFICATION / self.pixels_per_point).max(self.fit_scale())
    }

    /// A viewport's scale and center, with `Fit` worked out for this viewer.
    fn resolve(&self, viewport: Viewport) -> (f32, Pos2) {
        match viewport {
            Viewport::Fit => (self.fit_scale(), (self.image / 2.0).to_pos2()),
            Viewport::Zoomed { scale, center } => (scale, center),
        }
    }

    /// Where the image is drawn, in points.
    pub fn image_rect(&self, viewport: Viewport) -> Rect {
        let (scale, center) = self.resolve(viewport);
        Rect::from_min_size(
            self.rect.center() - center.to_vec2() * scale,
            self.image * scale,
        )
    }

    /// The point on the image, in image pixels, that's shown at `position` in the viewer.
    pub fn image_point(&self, viewport: Viewport, position: Pos2) -> Pos2 {
        let (scale, center) = self.resolve(viewport);
        center + (position - self.rect.center()) / scale
    }

    /// Zooms in by `factor` (or out, if it's less than 1), keeping the point on the image under
    /// `anchor` where it is, as far as the image's bounds allow.
    pub fn zoom(&self, viewport: Viewport, factor: f32, anchor: Pos2) -> Viewport {
        let (scale, _) = self.resolve(viewport);
        self.zoom_to(viewport, scale * factor, anchor)
    }

    /// Zooms to `scale`, keeping the point on the image under `anchor` where it is, as far as
    /// the image's bounds allow.
    pub fn zoom_to(&self, viewport: Viewport, scale: f32, anchor: Pos2) -> Viewport {
        // Limit magnification before working out the center, so pinching past the limit
        // doesn't slide the image around.
        let scale = scale.min(self.max_scale());
        self.clamp(self.zoom_to_unclamped(viewport, scale, anchor))
    }

    fn zoom_to_unclamped(&self, viewport: Viewport, scale: f32, anchor: Pos2) -> Viewport {
        let point = self.image_point(viewport, anchor);
        let center = point - (anchor - self.rect.center()) / scale;
        Viewport::Zoomed { scale, center }
    }

    /// Moves the image by `delta` points, as far as its bounds allow.
    pub fn pan(&self, viewport: Viewport, delta: Vec2) -> Viewport {
        self.clamp(self.pan_unclamped(viewport, delta))
    }

    fn pan_unclamped(&self, viewport: Viewport, delta: Vec2) -> Viewport {
        let (scale, center) = self.resolve(viewport);
        Viewport::Zoomed {
            scale,
            center: center - delta / scale,
        }
    }

    /// Keeps a viewport in bounds. The image is never smaller than fitting, or magnified past
    /// [`MAX_MAGNIFICATION`]. Along each axis, an image bigger than the viewer covers it, with no
    /// empty space at the edges, and a smaller one is centered.
    pub fn clamp(&self, viewport: Viewport) -> Viewport {
        let Viewport::Zoomed { scale, center } = viewport else {
            return Viewport::Fit;
        };
        // With no room to show the image, there's nothing to zoom.
        if self.fit_scale() <= 0.0 {
            return Viewport::Fit;
        }
        let scale = scale.min(self.max_scale());
        // Zooming out to roughly the fitting size snaps to fitting, so the image keeps fitting
        // when the viewer is resized. This comes after limiting magnification, which can bring
        // the scale down to fitting when the image is tiny.
        if scale <= self.fit_scale() * 1.0001 {
            return Viewport::Fit;
        }
        // Half the viewer's size, in image pixels.
        let half_view = self.rect.size() / (2.0 * scale);
        let clamp_axis = |center: f32, half_view: f32, length: f32| {
            if 2.0 * half_view >= length {
                length / 2.0
            } else {
                center.clamp(half_view, length - half_view)
            }
        };
        Viewport::Zoomed {
            scale,
            center: Pos2::new(
                clamp_axis(center.x, half_view.x, self.image.x),
                clamp_axis(center.y, half_view.y, self.image.y),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;
    use hegel::{TestCase, generators as gs};

    #[hegel::composite]
    fn gen_viewer(tc: &TestCase) -> Viewer {
        let coordinate = || gs::floats::<f32>().min_value(-1e4).max_value(1e4);
        let length = |max| gs::floats::<f32>().min_value(1.0).max_value(max);
        let min = pos2(tc.draw(coordinate()), tc.draw(coordinate()));
        let size = Vec2::new(tc.draw(length(1e4)), tc.draw(length(1e4)));
        Viewer {
            rect: Rect::from_min_size(min, size),
            pixels_per_point: tc.draw(gs::sampled_from(vec![1.0, 1.5, 2.0, 3.0])),
            image: Vec2::new(tc.draw(length(2e4)), tc.draw(length(2e4))),
        }
    }

    /// Any viewport, in bounds or not.
    #[hegel::composite]
    fn gen_viewport(tc: &TestCase, viewer: Viewer) -> Viewport {
        if tc.draw(gs::booleans()) {
            return Viewport::Fit;
        }
        let scale = tc.draw(gs::floats::<f32>().min_value(1e-3).max_value(20.0));
        let x = tc.draw(gs::floats::<f32>().min_value(-viewer.image.x).max_value(2.0 * viewer.image.x));
        let y = tc.draw(gs::floats::<f32>().min_value(-viewer.image.y).max_value(2.0 * viewer.image.y));
        Viewport::Zoomed {
            scale,
            center: pos2(x, y),
        }
    }

    /// A position inside the viewer.
    #[hegel::composite]
    fn gen_position(tc: &TestCase, viewer: Viewer) -> Pos2 {
        let r = viewer.rect;
        pos2(
            tc.draw(gs::floats::<f32>().min_value(r.min.x).max_value(r.max.x)),
            tc.draw(gs::floats::<f32>().min_value(r.min.y).max_value(r.max.y)),
        )
    }

    /// How far off a computed coordinate can be from rounding: a few f32 steps at the largest
    /// coordinate involved.
    fn position_error(rects: &[Rect]) -> f32 {
        let magnitude = rects
            .iter()
            .flat_map(|r| [r.min.x, r.min.y, r.max.x, r.max.y])
            .fold(1.0, |max: f32, x| max.max(x.abs()));
        8.0 * f32::EPSILON * magnitude
    }

    fn close(a: f32, b: f32, absolute: f32) -> bool {
        (a - b).abs() <= absolute + 1e-4 * a.abs().max(b.abs())
    }

    fn close_pos(a: Pos2, b: Pos2, absolute: f32) -> bool {
        close(a.x, b.x, absolute) && close(a.y, b.y, absolute)
    }

    /// Asserts that two viewports show the same thing, up to rounding.
    fn assert_same_view(viewer: Viewer, a: Viewport, b: Viewport) {
        match (a, b) {
            (Viewport::Fit, Viewport::Fit) => {}
            (
                Viewport::Zoomed { scale: s1, center: c1 },
                Viewport::Zoomed { scale: s2, center: c2 },
            ) => {
                assert!(close(s1, s2, 0.0), "{a:?} != {b:?}");
                let error = position_error(&[viewer.rect]) / s1;
                assert!(close_pos(c1, c2, error), "{a:?} != {b:?}");
            }
            _ => panic!("{a:?} != {b:?}"),
        }
    }

    // Fitting

    #[hegel::test(test_cases = 10_000)]
    fn fit_stays_inside_the_viewer(tc: TestCase) {
        let viewer = tc.draw(gen_viewer());
        let rect = viewer.image_rect(Viewport::Fit);
        let error = position_error(&[viewer.rect]);
        assert!(viewer.rect.expand(error).contains_rect(rect), "{rect:?} in {viewer:?}");
    }

    #[hegel::test(test_cases = 10_000)]
    fn fit_keeps_the_aspect_ratio(tc: TestCase) {
        let viewer = tc.draw(gen_viewer());
        let rect = viewer.image_rect(Viewport::Fit);
        // Compare cross products rather than ratios, to avoid dividing by tiny heights.
        let error = position_error(&[viewer.rect]) * (viewer.image.x + viewer.image.y);
        let (lhs, rhs) = (rect.width() * viewer.image.y, rect.height() * viewer.image.x);
        assert!(close(lhs, rhs, error), "{rect:?} vs {viewer:?}");
    }

    #[hegel::test(test_cases = 10_000)]
    fn fit_is_centered(tc: TestCase) {
        let viewer = tc.draw(gen_viewer());
        let rect = viewer.image_rect(Viewport::Fit);
        let error = position_error(&[viewer.rect]);
        assert!(close_pos(rect.center(), viewer.rect.center(), error));
    }

    #[hegel::test(test_cases = 10_000)]
    fn fit_fills_one_dimension(tc: TestCase) {
        // Otherwise it could be bigger.
        let viewer = tc.draw(gen_viewer());
        let rect = viewer.image_rect(Viewport::Fit);
        let error = position_error(&[viewer.rect]);
        assert!(
            close(rect.width(), viewer.rect.width(), error)
                || close(rect.height(), viewer.rect.height(), error),
            "{rect:?} in {viewer:?}",
        );
    }

    #[hegel::test(test_cases = 10_000)]
    fn fit_is_never_inverted(tc: TestCase) {
        // A cramped layout can leave a negative amount of space. Then there's no room for the
        // image, and it mustn't be drawn mirrored instead.
        let side = || gs::floats::<f32>().min_value(-1e4).max_value(1e4);
        let mut viewer = tc.draw(gen_viewer());
        viewer.rect = Rect::from_min_size(
            viewer.rect.min,
            Vec2::new(tc.draw(side()), tc.draw(side())),
        );
        let rect = viewer.image_rect(Viewport::Fit);
        assert!(rect.width() >= 0.0 && rect.height() >= 0.0, "{rect:?}");
    }

    // Bounds

    #[hegel::test(test_cases = 10_000)]
    fn clamp_keeps_the_image_in_bounds(tc: TestCase) {
        let viewer = tc.draw(gen_viewer());
        let viewport = viewer.clamp(tc.draw(gen_viewport(viewer)));
        let Viewport::Zoomed { scale, .. } = viewport else {
            return;
        };
        assert!(scale > viewer.fit_scale(), "smaller than fitting");
        assert!(scale * viewer.pixels_per_point <= MAX_MAGNIFICATION * 1.0001, "too big");

        let rect = viewer.image_rect(viewport);
        let view = viewer.rect;
        let error = position_error(&[view, rect]);
        for (image_min, image_max, view_min, view_max) in [
            (rect.min.x, rect.max.x, view.min.x, view.max.x),
            (rect.min.y, rect.max.y, view.min.y, view.max.y),
        ] {
            if image_max - image_min > view_max - view_min {
                assert!(image_min <= view_min + error && image_max >= view_max - error);
            } else {
                assert!(close(image_min + image_max, view_min + view_max, 2.0 * error));
            }
        }
    }

    #[hegel::test(test_cases = 10_000)]
    fn clamp_is_idempotent(tc: TestCase) {
        let viewer = tc.draw(gen_viewer());
        let once = viewer.clamp(tc.draw(gen_viewport(viewer)));
        let twice = viewer.clamp(once);
        assert_same_view(viewer, once, twice);
    }

    #[hegel::test(test_cases = 10_000)]
    fn zooming_out_far_enough_fits(tc: TestCase) {
        let viewer = tc.draw(gen_viewer());
        let viewport = viewer.clamp(tc.draw(gen_viewport(viewer)));
        let anchor = tc.draw(gen_position(viewer));
        assert_eq!(viewer.zoom(viewport, 1e-6, anchor), Viewport::Fit);
    }

    #[hegel::test(test_cases = 10_000)]
    fn zooming_past_the_limit_leaves_the_image_still(tc: TestCase) {
        // Pinching further at maximum magnification mustn't slide the image around.
        let viewer = tc.draw(gen_viewer());
        let x = tc.draw(gs::floats::<f32>().min_value(0.0).max_value(viewer.image.x));
        let y = tc.draw(gs::floats::<f32>().min_value(0.0).max_value(viewer.image.y));
        let at_max = viewer.clamp(Viewport::Zoomed {
            scale: viewer.max_scale(),
            center: pos2(x, y),
        });
        let anchor = tc.draw(gen_position(viewer));
        let factor = tc.draw(gs::floats::<f32>().min_value(1.0).max_value(10.0));
        assert_same_view(viewer, viewer.zoom(at_max, factor, anchor), at_max);
    }

    #[hegel::test(test_cases = 10_000)]
    fn a_viewer_with_no_room_only_fits(tc: TestCase) {
        // With no room there's no sensible zoom, and dividing by a zero scale would leave a
        // broken viewport behind.
        let mut viewer = tc.draw(gen_viewer());
        let side = || gs::floats::<f32>().min_value(-1e4).max_value(0.0);
        viewer.rect = if tc.draw(gs::booleans()) {
            Rect::from_min_size(viewer.rect.min, Vec2::new(tc.draw(side()), viewer.rect.height()))
        } else {
            Rect::from_min_size(viewer.rect.min, Vec2::new(viewer.rect.width(), tc.draw(side())))
        };
        let viewport = tc.draw(gen_viewport(viewer));
        let anchor = viewer.rect.center();
        let scale = tc.draw(gs::floats::<f32>().min_value(1e-3).max_value(20.0));
        assert_eq!(viewer.clamp(viewport), Viewport::Fit);
        assert_eq!(viewer.zoom_to(viewport, scale, anchor), Viewport::Fit);
        assert_eq!(viewer.zoom_to(Viewport::Fit, scale, anchor), Viewport::Fit);
    }

    // Zooming and panning, before bounds are applied

    #[hegel::test(test_cases = 10_000)]
    fn zooming_keeps_the_point_under_the_anchor(tc: TestCase) {
        let viewer = tc.draw(gen_viewer());
        let viewport = tc.draw(gen_viewport(viewer));
        let anchor = tc.draw(gen_position(viewer));
        let scale = tc.draw(gs::floats::<f32>().min_value(1e-3).max_value(20.0));
        let zoomed = viewer.zoom_to_unclamped(viewport, scale, anchor);

        let before = viewer.image_point(viewport, anchor);
        let after = viewer.image_point(zoomed, anchor);
        let error = position_error(&[viewer.rect]) / scale.min(viewer.resolve(viewport).0);
        assert!(close_pos(before, after, error), "{before:?} != {after:?}");
    }

    #[hegel::test(test_cases = 10_000)]
    fn panning_moves_the_image_by_the_delta(tc: TestCase) {
        let viewer = tc.draw(gen_viewer());
        let viewport = tc.draw(gen_viewport(viewer));
        let coordinate = || gs::floats::<f32>().min_value(-1e3).max_value(1e3);
        let delta = Vec2::new(tc.draw(coordinate()), tc.draw(coordinate()));
        let before = viewer.image_rect(viewport);
        let after = viewer.image_rect(viewer.pan_unclamped(viewport, delta));
        let error = position_error(&[viewer.rect, before, after]);
        assert!(close_pos(after.min, before.min + delta, error), "{before:?} → {after:?}");
        assert!(close(after.width(), before.width(), error));
    }

    // 100%

    #[hegel::test(test_cases = 10_000)]
    fn actual_size_shows_one_image_pixel_per_screen_pixel(tc: TestCase) {
        let viewer = tc.draw(gen_viewer());
        let viewport = viewer.clamp(tc.draw(gen_viewport(viewer)));
        let anchor = tc.draw(gen_position(viewer));
        let actual = viewer.zoom_to(viewport, viewer.actual_scale(), anchor);
        // Unless 100% would be smaller than fitting, in which case it fits.
        if viewer.actual_scale() > viewer.fit_scale() * 1.0001 {
            let rect = viewer.image_rect(actual);
            let screen_pixels = rect.width() * viewer.pixels_per_point;
            let error = position_error(&[viewer.rect, rect]) * viewer.pixels_per_point;
            assert!(close(screen_pixels, viewer.image.x, error), "{screen_pixels}");
        } else {
            assert_eq!(actual, Viewport::Fit);
        }
    }
}

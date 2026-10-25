//! Zoom and pan rules for the image canvas.
//!
//! All lengths here are logical pixels except image pixel counts. The one rule
//! worth stating up front: **100% means one image pixel per *physical* pixel**,
//! so the logical scale for 100% is `1 / window_scale_factor`. That is what
//! makes a HiDPI display show the image crisply rather than blurrily scaled.

use crate::geom::{Point, Size};

/// The ladder the zoom-in/zoom-out commands walk through, as factors where
/// `1.0` is 100%.
pub const ZOOM_STEPS: &[f32] = &[
    0.05, 0.10, 0.125, 0.25, 0.333, 0.50, 0.666, 1.0, 1.25, 1.50, 1.75, 2.0, 2.5, 3.0, 4.0, 6.0,
    8.0, 12.0, 16.0, 24.0, 32.0,
];

/// Smallest and largest factors the ladder allows.
pub const MIN_FACTOR: f32 = ZOOM_STEPS[0];
pub const MAX_FACTOR: f32 = ZOOM_STEPS[ZOOM_STEPS.len() - 1];

/// Which dimension an "auto" fit is derived from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FitAxis {
    /// Fit both dimensions, i.e. the whole image is visible.
    #[default]
    Window,
    /// Match the viewport width; vertical scrolling is expected.
    Width,
    /// Match the viewport height; horizontal scrolling is expected.
    Height,
}

/// How the image is scaled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomMode {
    /// Fit to the window, but never enlarge past 100% unless stretching is on.
    Auto,
    /// An explicit factor, where `1.0` is 100%.
    Factor(f32),
}

impl Default for ZoomMode {
    fn default() -> Self {
        ZoomMode::Auto
    }
}

/// Resolved geometry for one frame, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewLayout {
    /// Logical pixels per image pixel.
    pub scale: f32,
    /// On-screen size of the whole image.
    pub drawn: Size,
    /// Top-left of the drawn image, relative to the viewport's top-left.
    pub origin: Point,
}

impl ViewLayout {
    /// The factor as a percentage, for display. `1.0` scale at scale factor 1
    /// reports 100.
    pub fn percent(&self, window_scale_factor: f32) -> f32 {
        self.scale * window_scale_factor * 100.
    }

    /// True when the drawn image is wider or taller than the viewport, so the
    /// scrollbars and side arrows are meaningful.
    pub fn overflows(&self, viewport: Size) -> bool {
        self.drawn.w > viewport.w + 0.5 || self.drawn.h > viewport.h + 0.5
    }
}

/// Zoom, fit and pan state for the canvas.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoomState {
    mode: ZoomMode,
    /// Which axis an `Auto` fit uses.
    pub fit_axis: FitAxis,
    /// When set, images smaller than the window are scaled up to fill it.
    pub stretch_small: bool,
    /// Image centre offset from the viewport centre, in logical pixels.
    /// `(0, 0)` is centred.
    offset: Point,
    /// When locked, zoom commands are ignored.
    pub zoom_locked: bool,
    /// When locked, pan commands are ignored.
    pub scroll_locked: bool,
}

impl Default for ZoomState {
    fn default() -> Self {
        Self {
            mode: ZoomMode::Auto,
            fit_axis: FitAxis::Window,
            stretch_small: false,
            offset: Point::ZERO,
            zoom_locked: false,
            scroll_locked: false,
        }
    }
}

impl ZoomState {
    pub fn mode(&self) -> ZoomMode {
        self.mode
    }

    pub fn offset(&self) -> Point {
        self.offset
    }

    /// The factor the user currently sees, where `1.0` is 100%.
    ///
    /// Needs the image and viewport because `Auto` derives it from them.
    pub fn effective_factor(&self, image: Size, viewport: Size) -> f32 {
        match self.mode {
            ZoomMode::Factor(f) => f,
            ZoomMode::Auto => self.auto_factor(image, viewport),
        }
    }

    /// Switch to an explicit factor, keeping the current pan.
    pub fn set_factor(&mut self, factor: f32) {
        self.mode = ZoomMode::Factor(factor.clamp(MIN_FACTOR, MAX_FACTOR));
    }

    /// Go back to fitting, and recentre.
    pub fn set_auto(&mut self, axis: FitAxis) {
        self.mode = ZoomMode::Auto;
        self.fit_axis = axis;
        self.offset = Point::ZERO;
    }

    pub fn set_auto_axis(&mut self, axis: FitAxis) {
        self.fit_axis = axis;
        if matches!(self.mode, ZoomMode::Auto) {
            self.offset = Point::ZERO;
        }
    }

    /// Recompute the auto fit from the image and viewport.
    pub fn refresh_auto(&mut self, image: Size, viewport: Size) {
        if matches!(self.mode, ZoomMode::Auto) {
            self.mode = ZoomMode::Factor(self.auto_factor(image, viewport));
        }
    }

    /// Step to the next larger ladder rung.
    pub fn zoom_in(&mut self, image: Size, viewport: Size) {
        if self.zoom_locked {
            return;
        }
        let current = self.effective_factor(image, viewport);
        self.set_factor(next_step_up(current));
    }

    /// Step to the next smaller ladder rung.
    pub fn zoom_out(&mut self, image: Size, viewport: Size) {
        if self.zoom_locked {
            return;
        }
        let current = self.effective_factor(image, viewport);
        self.set_factor(next_step_down(current));
    }

    /// Jump straight to 100%.
    pub fn zoom_actual_size(&mut self) {
        self.set_factor(1.0);
    }

    /// Zoom while holding `anchor` (a viewport position) still, which is what
    /// wheel-over-cursor zooming should feel like.
    ///
    /// The anchor is only preserved when the pan clamp does not bind; when the
    /// image is smaller than the viewport on an axis it must stay centred there.
    pub fn zoom_at(&mut self, anchor: Point, image: Size, viewport: Size, step: ZoomStep) {
        if self.zoom_locked || image.is_empty() || viewport.is_empty() {
            return;
        }

        let before = self.resolve(image, viewport, 1.0);
        let current = self.effective_factor(image, viewport);
        let target = match step {
            ZoomStep::In => next_step_up(current),
            ZoomStep::Out => next_step_down(current),
            ZoomStep::Factor(f) => f.clamp(MIN_FACTOR, MAX_FACTOR),
        };
        if (target - current).abs() < 1e-6 {
            return;
        }

        // Where the cursor sits in image space, using the geometry actually on
        // screen so a clamped pan does not throw the anchor off.
        let image_point = Point::new(
            (anchor.x - before.origin.x) / before.scale,
            (anchor.y - before.origin.y) / before.scale,
        );

        self.mode = ZoomMode::Factor(target);

        let after_scale = self.resolve_scale(image, viewport, 1.0);
        let drawn = image.scaled(after_scale);
        // `origin = (viewport - drawn) / 2 + offset`, so solve for the offset
        // that puts `image_point` back under the cursor.
        let origin_wanted = Point::new(
            anchor.x - image_point.x * after_scale,
            anchor.y - image_point.y * after_scale,
        );
        let offset_wanted = Point::new(
            origin_wanted.x - (viewport.w - drawn.w) / 2.,
            origin_wanted.y - (viewport.h - drawn.h) / 2.,
        );
        self.offset = self.clamp_offset(offset_wanted, drawn, viewport);
    }

    /// Pan by a logical-pixel delta.
    pub fn pan(&mut self, delta: Point, image: Size, viewport: Size, window_scale_factor: f32) {
        if self.scroll_locked {
            return;
        }
        let layout = self.resolve(image, viewport, window_scale_factor);
        let wanted = Point::new(self.offset.x + delta.x, self.offset.y + delta.y);
        self.offset = self.clamp_offset(wanted, layout.drawn, viewport);
    }

    /// Recentre the image.
    pub fn center(&mut self) {
        self.offset = Point::ZERO;
    }

    /// Move the view to the given fraction of the image, where `(0, 0)` is the
    /// top-left and `(1, 1)` the bottom-right. Used by the scroll-to-corner
    /// commands.
    pub fn scroll_to_fraction(
        &mut self,
        fx: f32,
        fy: f32,
        image: Size,
        viewport: Size,
        window_scale_factor: f32,
    ) {
        if self.scroll_locked {
            return;
        }
        let layout = self.resolve(image, viewport, window_scale_factor);
        let slack = Point::new(
            ((layout.drawn.w - viewport.w) / 2.).max(0.),
            ((layout.drawn.h - viewport.h) / 2.).max(0.),
        );
        self.offset = Point::new(
            slack.x * (fx * 2. - 1.),
            slack.y * (fy * 2. - 1.),
        );
    }

    /// Resolve the geometry to draw this frame.
    pub fn resolve(&self, image: Size, viewport: Size, window_scale_factor: f32) -> ViewLayout {
        if image.is_empty() || viewport.is_empty() {
            return ViewLayout {
                scale: 1.,
                drawn: Size::default(),
                origin: Point::ZERO,
            };
        }

        let scale = self.resolve_scale(image, viewport, window_scale_factor);
        let drawn = image.scaled(scale);
        let offset = self.clamp_offset(self.offset, drawn, viewport);

        ViewLayout {
            scale,
            drawn,
            origin: Point::new(
                (viewport.w - drawn.w) / 2. + offset.x,
                (viewport.h - drawn.h) / 2. + offset.y,
            ),
        }
    }

    /// Logical pixels per image pixel for the current mode.
    fn resolve_scale(&self, image: Size, viewport: Size, window_scale_factor: f32) -> f32 {
        let natural = 1. / window_scale_factor.max(f32::EPSILON);
        match self.mode {
            // An explicit factor is a percentage, where 100% means one image
            // pixel per *physical* pixel, so it must be divided by the window
            // scale factor to become logical pixels.
            ZoomMode::Factor(f) => f * natural,
            // A fit factor is already a logical-pixels-per-image-pixel ratio,
            // measured against the viewport. Dividing it again would apply the
            // DPI twice and leave the image short of the window.
            ZoomMode::Auto => self.auto_factor(image, viewport),
        }
    }

    /// The auto fit factor, expressed the same way the user sees it (1.0 is
    /// 100%), so it can be compared with ladder rungs.
    fn auto_factor(&self, image: Size, viewport: Size) -> f32 {
        if image.is_empty() || viewport.is_empty() {
            return 1.;
        }
        let fit = match self.fit_axis {
            FitAxis::Window => (viewport.w / image.w).min(viewport.h / image.h),
            FitAxis::Width => viewport.w / image.w,
            FitAxis::Height => viewport.h / image.h,
        };
        // An explicit axis means "fill that axis", so it always fills. Only the
        // automatic fit avoids enlarging, keeping small images pixel-exact
        // unless the user turns stretching on.
        let fit = match self.fit_axis {
            FitAxis::Width | FitAxis::Height => fit,
            FitAxis::Window if self.stretch_small => fit,
            FitAxis::Window => fit.min(1.),
        };
        fit.clamp(MIN_FACTOR, MAX_FACTOR)
    }

    /// Clamp a centre offset so the image cannot be dragged out of view: when
    /// it is larger than the viewport it may travel exactly to its edges, and
    /// when it is smaller it stays centred.
    fn clamp_offset(&self, offset: Point, drawn: Size, viewport: Size) -> Point {
        let max = Point::new(
            ((drawn.w - viewport.w) / 2.).max(0.),
            ((drawn.h - viewport.h) / 2.).max(0.),
        );
        offset.clamp_to(max)
    }
}

/// A requested zoom change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomStep {
    In,
    Out,
    Factor(f32),
}

/// Next rung above `current`, saturating at the top.
pub fn next_step_up(current: f32) -> f32 {
    ZOOM_STEPS
        .iter()
        .copied()
        .find(|step| *step > current + 1e-4)
        .unwrap_or(MAX_FACTOR)
}

/// Next rung below `current`, saturating at the bottom.
pub fn next_step_down(current: f32) -> f32 {
    ZOOM_STEPS
        .iter()
        .copied()
        .rev()
        .find(|step| *step < current - 1e-4)
        .unwrap_or(MIN_FACTOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> Size {
        Size::new(1000., 500.)
    }

    fn viewport() -> Size {
        Size::new(500., 500.)
    }

    #[test]
    fn auto_fits_large_image_and_centres_it() {
        let state = ZoomState::default();
        let layout = state.resolve(image(), viewport(), 1.0);

        // Height is the binding constraint: 500/500 = 0.5.
        assert!((layout.scale - 0.5).abs() < 1e-5, "scale was {}", layout.scale);
        assert_eq!(layout.drawn, Size::new(500., 250.));
        assert_eq!(layout.origin, Point::new(0., 125.));
    }

    #[test]
    fn auto_does_not_enlarge_small_images() {
        let state = ZoomState::default();
        let small = Size::new(100., 100.);
        let layout = state.resolve(small, viewport(), 1.0);

        assert!((layout.scale - 1.0).abs() < 1e-5);
        assert_eq!(layout.drawn, small);
        // Centred.
        assert_eq!(layout.origin, Point::new(200., 200.));
    }

    #[test]
    fn stretch_small_enlarges_to_fill() {
        let mut state = ZoomState::default();
        state.stretch_small = true;
        let layout = state.resolve(Size::new(100., 100.), viewport(), 1.0);

        assert!((layout.scale - 5.0).abs() < 1e-5);
        assert_eq!(layout.drawn, Size::new(500., 500.));
    }

    #[test]
    fn hundred_percent_maps_image_pixels_to_physical_pixels() {
        // On a 2x display, 100% must mean one image pixel per *physical* pixel,
        // so the logical scale is 0.5 and the drawn size halves.
        let mut state = ZoomState::default();
        state.set_factor(1.0);
        let layout = state.resolve(image(), viewport(), 2.0);

        assert!((layout.scale - 0.5).abs() < 1e-5);
        assert_eq!(layout.drawn, Size::new(500., 250.));
        assert!((layout.percent(2.0) - 100.).abs() < 1e-3);
    }

    #[test]
    fn fit_axes_pick_the_right_dimension() {
        let mut state = ZoomState::default();

        state.set_auto(FitAxis::Width);
        assert!((state.resolve(image(), viewport(), 1.0).scale - 0.5).abs() < 1e-5);

        state.set_auto(FitAxis::Height);
        // 500/500 = 1.0.
        assert!((state.resolve(image(), viewport(), 1.0).scale - 1.0).abs() < 1e-5);
    }

    #[test]
    fn pan_is_clamped_to_the_image_edges() {
        let mut state = ZoomState::default();
        state.set_factor(2.0);
        // Drawn 2000x1000 in a 500x500 viewport, so the slack is 750 and 250.
        state.pan(Point::new(10_000., 10_000.), image(), viewport(), 1.0);
        assert_eq!(state.offset(), Point::new(750., 250.));

        state.pan(Point::new(-10_000., -10_000.), image(), viewport(), 1.0);
        assert_eq!(state.offset(), Point::new(-750., -250.));

        // Fully panned right/down puts the image's top-left at the viewport's.
        state.pan(Point::new(10_000., 10_000.), image(), viewport(), 1.0);
        assert_eq!(state.resolve(image(), viewport(), 1.0).origin, Point::new(0., 0.));
    }

    #[test]
    fn small_image_cannot_be_panned_off_centre() {
        let mut state = ZoomState::default();
        state.set_factor(0.25);
        state.pan(Point::new(500., 500.), image(), viewport(), 1.0);
        assert_eq!(state.offset(), Point::ZERO, "should stay centred");

        // 250x125 centred in 500x500.
        assert_eq!(
            state.resolve(image(), viewport(), 1.0).origin,
            Point::new(125., 187.5)
        );
    }

    #[test]
    fn scroll_fractions_reach_the_corners() {
        let mut state = ZoomState::default();
        state.set_factor(2.0);

        state.scroll_to_fraction(0., 0., image(), viewport(), 1.0);
        assert_eq!(state.offset(), Point::new(-750., -250.));

        state.scroll_to_fraction(1., 1., image(), viewport(), 1.0);
        assert_eq!(state.offset(), Point::new(750., 250.));

        state.scroll_to_fraction(0.5, 0.5, image(), viewport(), 1.0);
        assert_eq!(state.offset(), Point::ZERO);
    }

    #[test]
    fn wheel_zoom_keeps_the_point_under_the_cursor() {
        // A square image that overflows the viewport at both zoom levels, and
        // an anchor far enough from the edge that the pan clamp does not bind,
        // so the anchor invariant is exact.
        let big = Size::new(2000., 2000.);
        let mut state = ZoomState::default();
        state.set_factor(0.5);
        let anchor = Point::new(200., 350.);

        let before = state.resolve(big, viewport(), 1.0);
        let image_point = Point::new(
            (anchor.x - before.origin.x) / before.scale,
            (anchor.y - before.origin.y) / before.scale,
        );

        state.zoom_at(anchor, big, viewport(), ZoomStep::In);

        let after = state.resolve(big, viewport(), 1.0);
        assert!(after.scale > before.scale, "should have zoomed in");
        let landed = Point::new(
            after.origin.x + image_point.x * after.scale,
            after.origin.y + image_point.y * after.scale,
        );
        assert!(
            (landed.x - anchor.x).abs() < 0.5 && (landed.y - anchor.y).abs() < 0.5,
            "cursor drifted from {anchor:?} to {landed:?}"
        );
    }

    #[test]
    fn wheel_zoom_recentres_an_axis_that_no_longer_overflows() {
        // Zooming out until the image is shorter than the viewport must leave
        // it centred vertically, even though the anchor wanted otherwise.
        let mut state = ZoomState::default();
        state.set_factor(1.0);
        state.zoom_at(Point::new(10., 10.), image(), viewport(), ZoomStep::Out);

        let layout = state.resolve(image(), viewport(), 1.0);
        assert_eq!(layout.origin.y, (viewport().h - layout.drawn.h) / 2.);
    }

    #[test]
    fn zoom_ladder_walks_and_saturates() {
        assert_eq!(next_step_up(1.0), 1.25);
        assert_eq!(next_step_down(1.0), 0.666);
        assert_eq!(next_step_up(MAX_FACTOR), MAX_FACTOR);
        assert_eq!(next_step_down(MIN_FACTOR), MIN_FACTOR);
        // A factor between rungs snaps to the neighbouring rung.
        assert_eq!(next_step_up(1.1), 1.25);
        assert_eq!(next_step_down(1.1), 1.0);
    }

    #[test]
    fn zoom_in_from_auto_leaves_auto() {
        let mut state = ZoomState::default();
        state.zoom_in(image(), viewport());
        // Auto fit was 0.5, so the next rung up is 0.666.
        assert_eq!(state.mode(), ZoomMode::Factor(0.666));
    }

    #[test]
    fn locks_suppress_zoom_and_pan() {
        let mut state = ZoomState::default();
        state.zoom_locked = true;
        state.scroll_locked = true;
        state.set_factor(2.0);
        let before = state.clone();

        state.zoom_in(image(), viewport());
        state.zoom_out(image(), viewport());
        state.pan(Point::new(100., 100.), image(), viewport(), 1.0);
        state.scroll_to_fraction(1., 1., image(), viewport(), 1.0);

        assert_eq!(state.offset(), before.offset());
        assert_eq!(state.mode(), before.mode());
    }

    #[test]
    fn empty_inputs_resolve_to_nothing_drawable() {
        let state = ZoomState::default();
        let layout = state.resolve(Size::default(), viewport(), 1.0);
        assert!(layout.drawn.is_empty());

        let layout = state.resolve(image(), Size::default(), 1.0);
        assert!(layout.drawn.is_empty());
    }

    #[test]
    fn auto_fit_is_not_scaled_by_dpi_a_twice() {
        // Regression: on a 2x display an auto fit must still fill the viewport.
        let state = ZoomState::default();
        let layout = state.resolve(image(), viewport(), 2.0);

        // Fit is 500/1000 = 0.5 of the image, in logical pixels. The test
        // image is wide, so fitting the window binds on width.
        assert!((layout.scale - 0.5).abs() < 1e-5, "scale was {}", layout.scale);
        assert!(
            (layout.drawn.w - viewport().w).abs() < 0.5,
            "should fill the width, got {}",
            layout.drawn.w
        );
    }

    #[test]
    fn explicit_axes_fill_even_small_images() {
        let small = Size::new(100., 100.);
        let mut state = ZoomState::default();

        state.set_auto(FitAxis::Height);
        let layout = state.resolve(small, viewport(), 1.0);
        assert!((layout.drawn.h - viewport().h).abs() < 0.5, "fit to height must fill");

        state.set_auto(FitAxis::Width);
        let layout = state.resolve(small, viewport(), 1.0);
        assert!((layout.drawn.w - viewport().w).abs() < 0.5, "fit to width must fill");
    }

    #[test]
    fn overflow_flag_tracks_the_viewport() {
        let mut state = ZoomState::default();
        state.set_factor(2.0);
        assert!(state.resolve(image(), viewport(), 1.0).overflows(viewport()));

        state.set_factor(0.25);
        assert!(!state.resolve(image(), viewport(), 1.0).overflows(viewport()));
    }
}

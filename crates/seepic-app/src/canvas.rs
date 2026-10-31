//! The image canvas: the image itself, the empty and loading states, the side
//! arrows and the zoom readout.
//!
//! Three browse modes draw here. Single-page modes position one image with the
//! zoom/pan state; two-page modes draw the pages of the current spread side by
//! side at a shared scale; webtoon stacks every page in a virtual list. They
//! share the same background treatment, which is what makes a transparent PNG
//! look the same in all three.

use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use gpui_kit::ExternalPaths;
use seepic_core::command::Command;

use crate::app::ViewerApp;

/// The transparent monochrome mark the empty page shows, derived from the
/// square app icon and embedded so the build has no runtime asset directory to
/// find.
const EMPTY_MARK_PNG: &[u8] = include_bytes!("../../../assets/app-icon-mark.png");
use crate::strings as s;

/// How far the side arrows sit from the edges.
const ARROW_INSET: f32 = 12.;
/// How tall the side arrows are; they are centred vertically.
///
/// The same height is the band along each edge that reveals them, so the button
/// appears when the pointer is level with it and nowhere else on the edge.
pub const ARROW_HEIGHT: f32 = 132.;

pub fn render(view: &mut ViewerApp, window: &Window, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    // A spread is two pages, and only the cursor's page is decoded for the main
    // view, so the partner is requested here: the render pass is the only place
    // that knows which spread is on screen.
    if view.view_mode.is_two_page() {
        view.ensure_spread_renders(cx);
    }
    // Landing in webtoon on the page the reader was already on needs the list's
    // layout, so the scroll is deferred to here rather than done at the switch.
    if let Some(target) = view.webtoon_pending_scroll.take() {
        view.webtoon_scroll
            .scroll_to_item(target, ScrollStrategy::Top);
    }

    // The configured background picture is read on the first frame that needs
    // it rather than at start-up, so a bad path cannot delay the window.
    view.ensure_background_image(cx);
    let backdrop_image = view.backdrop_image();

    let canvas = div()
        .id("canvas")
        // A native file drag arrives as an internal drag whose payload is the
        // dropped paths, so this handles "drag a file onto the window".
        .on_drop(cx.listener(|this, paths: &ExternalPaths, _window, cx| {
            this.accept_dropped_paths(paths.paths().to_vec(), cx);
        }))
        .size_full()
        .relative()
        .overflow_hidden()
        .bg(view.backdrop_color(cx))
        .cursor(CursorStyle::Arrow)
        .on_scroll_wheel(cx.listener(ViewerApp::on_scroll_wheel))
        .on_mouse_down(MouseButton::Left, cx.listener(ViewerApp::on_canvas_mouse_down))
        .on_mouse_up(MouseButton::Left, cx.listener(ViewerApp::on_canvas_mouse_up))
        .on_mouse_move(cx.listener(ViewerApp::on_canvas_mouse_move))
        .on_mouse_down(MouseButton::Middle, cx.listener(ViewerApp::on_canvas_mouse_down))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(ViewerApp::on_canvas_context_menu),
        )
        // Cover, not fill: a background picture must never be distorted, and
        // the Blur mode relies on the magnification this gives a tiny image.
        .when_some(backdrop_image, |this, image| {
            this.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .child(
                        img(gpui::ImageSource::Render(image))
                            .size_full()
                            .object_fit(ObjectFit::Cover),
                    ),
            )
        });

    let body = match (&view.current, view.loading) {
        (Some(_), _) if view.view_mode.is_webtoon() => webtoon(view, window, cx),
        (Some(_), _) if view.view_mode.is_two_page() => two_page(view, window, cx),
        (Some(image), _) => {
            let viewport = view.viewport(window);
            let layout = view
                .zoom
                .resolve(image.size, viewport, window.scale_factor());

            // The page-turn effect squashes, slides or fades the image.
            let (opacity, x_shift, width_scale) = view.page_turn_transform(viewport.w);
            let drawn_width = (layout.drawn.w * width_scale).max(1.);
            let left = layout.origin.x + x_shift + (layout.drawn.w - drawn_width) / 2.;

            div()
                .absolute()
                .left(px(left))
                .top(px(layout.origin.y))
                .w(px(drawn_width))
                .h(px(layout.drawn.h.max(1.)))
                .opacity(opacity)
                // A hairline around the image, which is what keeps a photo that
                // matches the background colour from bleeding into it.
                .when_some(view.border_rgb(), |this, colour| {
                    this.border_1().border_color(rgb(colour))
                })
                .child(
                    // Draw a mip level when zoomed out, so downscaling stays
                    // sharp instead of aliasing on the GPU's 4-texel filter.
                    //
                    // The id is what gives GPUI an animation state to advance:
                    // without one it draws frame zero for ever, however many
                    // frames the image carries. (The viewer drives long
                    // animations itself; see `ViewerApp`'s animation section.)
                    img(gpui::ImageSource::Render(
                        view.render_for_scale(layout.scale, window.scale_factor())
                            .map(|(render, _)| render.clone())
                            .unwrap_or_else(|| image.render.clone()),
                    ))
                    .id("canvas-image")
                    .size_full()
                    .object_fit(ObjectFit::Fill),
                )
                .into_any_element()
        }
        (None, true) => centered(loading_state(view, cx)).into_any_element(),
        (None, false) => match &view.error {
            Some(error) => centered(
                v_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .size(px(40.))
                            .text_color(cx.theme().danger),
                    )
                    .child(div().text_sm().max_w(px(420.)).child(error.clone())),
            )
            .into_any_element(),
            None => empty_state(view, cx).into_any_element(),
        },
    };

    canvas
        .child(body)
        // The paging arrows only appear near the edges, so the image is
        // unobstructed the rest of the time.
        .when(
            view.paging_arrows_visible(window) && view.view_mode.is_single_page(),
            |this| {
                let height = view.viewport(window).h;
                this.child(side_arrow(
                    IconName::ChevronLeft,
                    "prev",
                    Command::PrevImage,
                    height,
                    cx,
                ))
                .child(side_arrow(
                    IconName::ChevronRight,
                    "next",
                    Command::NextImage,
                    height,
                    cx,
                ))
            },
        )
        .when(view.anim_bar_shown(), |this| {
            this.child(anim_info(view, cx))
        })
}

/// The two-page spread: each page of the current pair, side by side.
fn two_page(view: &ViewerApp, window: &Window, cx: &mut Context<ViewerApp>) -> AnyElement {
    let Some(image) = &view.current else {
        return div().into_any_element();
    };
    let spread = view.current_spread();
    let viewport = view.viewport(window);
    if spread.size.is_empty() || viewport.is_empty() {
        return div().into_any_element();
    }

    // Both pages share one scale, so the spread fits as a unit and the pages
    // line up along the bottom edge.
    let scale = (viewport.w / spread.size.w)
        .min(viewport.h / spread.size.h)
        .min(1.)
        .max(0.01);
    let drawn = spread.size.scaled(scale);
    let origin = seepic_core::Point::new(
        (viewport.w - drawn.w) / 2.,
        (viewport.h - drawn.h) / 2.,
    );

    let mut container = div()
        .absolute()
        .left(px(origin.x))
        .top(px(origin.y))
        .w(px(drawn.w))
        .h(px(drawn.h));

    let current_position = view.navigator.position_of_source(image.index);

    for (position, page) in spread.pages.iter().enumerate() {
        let offset = spread.offsets.get(position).copied().unwrap_or(0.);
        let page_size = view
            .page_sizes
            .get(*page)
            .copied()
            .flatten()
            .unwrap_or(image.size);
        // The decoded image carries a container index while the spread is
        // indexed by display position, and sorting makes the two differ.
        let is_current = Some(*page) == current_position;
        let width = page_size.w * scale;
        let height = page_size.h * scale;

        container = container.child(
            div()
                .absolute()
                .left(px(offset * scale))
                .top(px((drawn.h - height).max(0.)))
                .w(px(width.max(1.)))
                .h(px(height.max(1.)))
                .when(is_current, |this| {
                    this.child(
                        img(gpui::ImageSource::Render(image.render.clone()))
                            .size_full()
                            .object_fit(ObjectFit::Fill),
                    )
                })
                .when(!is_current, |this| match view.spread_render(*page) {
                    Some(render) => this.child(
                        img(gpui::ImageSource::Render(render))
                            .size_full()
                            .object_fit(ObjectFit::Fill),
                    ),
                    // Stands in only until the partner page has been decoded.
                    None => this.flex().items_center().justify_center().child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("…"),
                    ),
                }),
        );
    }

    container.into_any_element()
}

/// The webtoon: every page stacked, scrolled continuously.
fn webtoon(view: &ViewerApp, window: &Window, cx: &mut Context<ViewerApp>) -> AnyElement {
    let viewport = view.viewport(window);
    let sizes = view.webtoon_item_sizes(viewport.w);
    if sizes.is_empty() {
        return div().into_any_element();
    }

    let item_sizes: Rc<Vec<gpui::Size<Pixels>>> = Rc::new(
        sizes
            .iter()
            .map(|item| gpui::size(px(item.w), px(item.h)))
            .collect(),
    );
    let entity = cx.entity();

    v_virtual_list(
        entity,
        "webtoon",
        item_sizes,
        |view, range, _window, cx| {
            let cursor = view.cursor_position();
            range
                .map(|index| {
                    // The render pass is the only place that knows which pages
                    // are visible, so requests are made from here.
                    view.ensure_webtoon_page(index, cx);
                    let is_current = index == cursor;
                    let render = view.webtoon_render(index);
                    div()
                        .id(("webtoon-page", index))
                        .w_full()
                        .h_full()
                        .flex()
                        .justify_center()
                        .when(is_current, |this| this.child(div().absolute()))
                        .when_some(render, |this, render| {
                            this.child(
                                img(gpui::ImageSource::Render(render))
                                    .size_full()
                                    .object_fit(ObjectFit::Fill),
                            )
                        })
                        .when(render_is_missing(view, index), |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("…"),
                            )
                        })
                        .into_any_element()
                })
                .collect()
        },
    )
    .track_scroll(&view.webtoon_scroll)
    .into_any_element()
}

/// Whether the page at `index` is still waiting to be decoded.
fn render_is_missing(view: &ViewerApp, index: usize) -> bool {
    view.webtoon_render(index).is_none()
}

/// The bottom strip that reports animation frames.
///
/// It is translucent because it sits over the picture, and the viewer draws it
/// only while the idle countdown says the controls are still wanted (see
/// `ViewerApp::anim_bar_shown`).
fn anim_info(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let total = view.frame_count();
    let current = view.held_frame().unwrap_or(0) + 1;
    let playing = view.anim_playing();

    let control = |id: &'static str, icon: IconName, command: Command, cx: &mut Context<ViewerApp>| {
        let theme = theme.clone();
        div()
            .id(id)
            .px_1p5()
            .py_1()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(move |this| this.bg(theme.button_hover))
            .child(Icon::new(icon).size(px(14.)))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.execute(command, window, cx);
            }))
    };

    div()
        .absolute()
        .bottom(px(4.))
        .left(px(0.))
        .right(px(0.))
        .flex()
        .justify_center()
        .child(
            h_flex()
                .id("anim-bar")
                .gap_1()
                .items_center()
                .px_2()
                .py_0p5()
                .rounded(px(8.))
                .bg(theme.secondary.opacity(0.82))
                .text_color(theme.secondary_foreground)
                .text_xs()
                // Resting on the controls keeps them up; the idle timer watches
                // this flag instead of hiding them under the pointer.
                .on_hover(cx.listener(|this, hovered: &bool, _window, cx| {
                    this.set_anim_bar_hovered(*hovered, cx);
                }))
                .child(control("anim-first", IconName::SkipBack, Command::AnimHome, cx))
                .child(control("anim-prev", IconName::ChevronLeft, Command::AnimPrev, cx))
                .child(control(
                    "anim-play",
                    if playing {
                        IconName::Pause
                    } else {
                        IconName::Play
                    },
                    Command::AnimPlayPause,
                    cx,
                ))
                .child(control("anim-next", IconName::ChevronRight, Command::AnimNext, cx))
                .child(control("anim-last", IconName::SkipForward, Command::AnimEnd, cx))
                .child(
                    div()
                        .px_1()
                        .child(format!("{current} / {total}")),
                ),
        )
}

/// The average of the two checker squares, which is what a viewer perceives at
/// a distance. The squares themselves are drawn over it.
pub const CHECKER_BASE: u32 = 0x5a5a5a;

fn centered(child: impl IntoElement) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(child)
}

/// The loading state, which reports how far a page-dimension probe has got.
fn loading_state(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    let theme = cx.theme().clone();
    let text = if view.probing {
        "正在读取图像信息…".to_string()
    } else {
        s::LOADING.to_string()
    };
    v_flex()
        .items_center()
        .gap_2()
        .child(div().text_sm().child(text))
        .when(view.probing, |this| {
            this.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("双页与漫画模式需要每张图片的尺寸"),
            )
        })
}

/// The application mark for the empty page: the icon's artwork with its
/// background removed, tinted light for the dark theme and dark for the light
/// one.
///
/// The source is a coverage mask, so the mark keeps the icon's internal shading
/// as varying opacity. Decoding it once and keeping both tints avoids rebuilding
/// an image on every render.
fn empty_mark(dark: bool) -> Option<Arc<RenderImage>> {
    static MARKS: OnceLock<[Option<Arc<RenderImage>>; 2]> = OnceLock::new();
    let marks = MARKS.get_or_init(|| {
        let Ok(mask) = image::load_from_memory(EMPTY_MARK_PNG).map(|image| image.to_luma8())
        else {
            return [None, None];
        };
        let (width, height) = mask.dimensions();
        let build = |rgb: [u8; 3]| -> Option<Arc<RenderImage>> {
            let mut rgba = Vec::with_capacity(mask.as_raw().len() * 4);
            for coverage in mask.as_raw() {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], *coverage]);
            }
            let buffer = image::RgbaImage::from_raw(
                width,
                height,
                seepic_core::decode::rgba_to_bgra(rgba),
            )?;
            let frame = image::Frame::from_parts(
                buffer,
                0,
                0,
                image::Delay::from_numer_denom_ms(100, 1),
            );
            Some(Arc::new(RenderImage::new(vec![frame])))
        };
        [build([245, 245, 245]), build([25, 25, 25])]
    });
    marks[if dark { 0 } else { 1 }].clone()
}

/// The placeholder shown when nothing is open.
fn empty_state(view: &ViewerApp, cx: &mut Context<ViewerApp>) -> impl IntoElement {
    // Contrast follows the canvas the mark is drawn on, not the chrome: a dark
    // custom background under a light theme still needs a light mark.
    let dark = view.backdrop_color(cx).l < 0.5;
    let mark = empty_mark(dark);

    centered(
        div()
            .w(px(180.))
            .h(px(180.))
            .when_some(mark, |this, mark| {
                this.child(
                    img(ImageSource::Render(mark))
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        // The same weight as the old placeholder: the empty
                        // page should not compete with an image.
                        .opacity(0.42),
                )
            }),
    )
}

/// A large, low-contrast arrow on each side for paging through images.
///
/// The button has no background of its own: over a photograph a grey plate is
/// more distracting than the arrow, and the arrow only appears while the pointer
/// is in its band anyway. Hovering brightens the arrow instead.
fn side_arrow(
    icon: IconName,
    id: &'static str,
    command: Command,
    viewport_height: f32,
    cx: &mut Context<ViewerApp>,
) -> impl IntoElement {
    let is_left = matches!(command, Command::PrevImage);
    let theme = cx.theme().clone();
    let color = theme.muted_foreground;

    div()
        .id(id)
        .absolute()
        // A centred band rather than the whole edge: a full-height button would
        // turn every click near the edge into a page turn.
        .top(px(((viewport_height - ARROW_HEIGHT) / 2.).max(0.)))
        .h(px(ARROW_HEIGHT))
        .when(is_left, |this| this.left(px(ARROW_INSET)))
        .when(!is_left, |this| this.right(px(ARROW_INSET)))
        .w(px(40.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_color(color)
        .hover(move |this| this.text_color(theme.foreground))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.execute(command, window, cx);
        }))
        // Two quick clicks here are two page turns, not a double click on the
        // image: the root's double-click detection would otherwise maximise the
        // window on the second release. The press reached the canvas and armed a
        // window drag, so that is cleared here as well.
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                this.chrome_drag_armed = false;
                cx.stop_propagation();
            }),
        )
        .child(Icon::new(icon).size(px(28.)))
}

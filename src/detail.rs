//! When a zoomed-in picture needs a sharper piece, and which piece.
//!
//! The picture on screen is decoded to fit the window with room to zoom
//! (`hyprforge_image`'s budget), and past that room it is only enlarged —
//! soft at 1:1 on any photograph bigger than twice the window. So once a
//! zoom settles, the window asks `hyprforge_image::decode_region` for the
//! part it can see, at the density it draws it at, and lays that over the
//! enlarged picture.
//!
//! Pure, like [`crate::transform`], so "is a sharper piece worth
//! decoding" is arithmetic a test can ask. This is the one module beside
//! the budget that takes the output's scale: the question is about
//! physical pixels, because that is what a picture looks soft in.

use crate::transform::{ImageSize, Transform, Viewport};
use hyprforge_image::Region;

/// How far past what is visible a piece reaches on each side, as a
/// fraction of the visible size — so a short pan stays sharp without
/// waiting for another decode. A half each way doubles each edge, which
/// is the most the budget keeps for a picture anyway (twice the window's
/// longest edge), so a piece retains no more than the fitted picture.
pub const MARGIN: f32 = 0.5;

/// How much sharper a piece has to be than what is on screen to be worth
/// a decode: a whole decode of the file is the price, so a piece that
/// would be 2% sharper is not asked for.
const WORTH_IT: f32 = 1.02;

/// A sharper piece the window wants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wanted {
    /// The piece to decode, in the picture's own pixels as presented —
    /// the user's turns applied — margin included.
    pub region: Region,
    /// Just what is on screen, which a piece already held must cover to
    /// make this one unnecessary.
    pub visible: Region,
    /// Physical pixels per picture pixel, at most 1.0 — past that a
    /// renderer enlarges, and decoding more would invent pixels.
    pub density: f32,
}

impl Wanted {
    /// Whether a piece already held — `region` at `density` — shows all
    /// this one would: it covers what is visible and is near enough as
    /// sharp. A pan inside the margin, or a zoom back out a step, then
    /// costs nothing.
    pub fn met_by(&self, region: Region, density: f32) -> bool {
        let v = self.visible;
        region.x <= v.x
            && region.y <= v.y
            && region.x + region.width >= v.x + v.width
            && region.y + region.height >= v.y + v.height
            && density * WORTH_IT >= self.density
    }
}

/// The piece wanted for `image` placed by `transform` in `viewport`,
/// drawn on an output of `scale_factor`, when the decode on screen holds
/// `decoded_density` physical pixels per picture pixel. `None` when that
/// decode is already as sharp as the screen can show — which is always
/// the case at fit, and for any picture small enough to be decoded whole.
pub fn wanted(
    transform: &Transform,
    image: ImageSize,
    viewport: Viewport,
    scale_factor: f32,
    decoded_density: f32,
) -> Option<Wanted> {
    let scale = transform.scale(image, viewport);
    let density = (scale * scale_factor).min(1.0);
    if !density.is_finite() || decoded_density * WORTH_IT >= density {
        return None;
    }
    let (drawn_w, drawn_h) = transform.drawn_size(image, viewport);
    let at = transform.top_left(image, viewport);
    // What of the picture the window shows, in logical pixels...
    let (x0, x1) = (at.x.max(0.0), (at.x + drawn_w).min(viewport.width));
    let (y0, y1) = (at.y.max(0.0), (at.y + drawn_h).min(viewport.height));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    // ...and in the picture's own.
    let (ix0, ix1) = ((x0 - at.x) / scale, (x1 - at.x) / scale);
    let (iy0, iy1) = ((y0 - at.y) / scale, (y1 - at.y) / scale);
    let visible = region_of(ix0, iy0, ix1, iy1, image);
    let (mx, my) = ((ix1 - ix0) * MARGIN, (iy1 - iy0) * MARGIN);
    let region = region_of(ix0 - mx, iy0 - my, ix1 + mx, iy1 + my, image);
    Some(Wanted { region, visible, density })
}

/// The whole pixels covering a rectangle, cut to the picture: outward to
/// the next pixel, so a sliver at an edge is never left out.
fn region_of(x0: f32, y0: f32, x1: f32, y1: f32, image: ImageSize) -> Region {
    let (w, h) = (image.width.max(1.0), image.height.max(1.0));
    let x = x0.clamp(0.0, w - 1.0).floor();
    let y = y0.clamp(0.0, h - 1.0).floor();
    let right = x1.clamp(x + 1.0, w).ceil();
    let bottom = y1.clamp(y + 1.0, h).ceil();
    Region { x: x as u32, y: y as u32, width: (right - x) as u32, height: (bottom - y) as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::Zoom;

    /// An 8000x4000 photograph in a 1000x800 window, decoded at 2000
    /// wide — a quarter of its pixels each way, as the budget keeps it.
    const IMAGE: ImageSize = ImageSize { width: 8000.0, height: 4000.0 };
    const VIEWPORT: Viewport = Viewport { width: 1000.0, height: 800.0 };
    const DECODED: f32 = 0.25;

    fn at(zoom: f32) -> Transform {
        Transform { zoom: Zoom::Factor(zoom), ..Transform::default() }
    }

    /// Fit is what the decode was made for; nothing sharper is wanted.
    #[test]
    fn at_fit_nothing_sharper_is_wanted() {
        assert_eq!(wanted(&Transform::default(), IMAGE, VIEWPORT, 1.0, DECODED), None);
    }

    /// The bug this exists for: at 1:1 the window shows the photograph's
    /// own pixels, and the decode has a quarter of them. What is wanted
    /// is the middle of the picture at full density, reaching past the
    /// window by the margin.
    #[test]
    fn at_one_to_one_the_visible_middle_is_wanted_at_full_density() {
        let w = wanted(&at(1.0), IMAGE, VIEWPORT, 1.0, DECODED).unwrap();
        assert_eq!(w.density, 1.0);
        assert_eq!(w.visible, Region { x: 3500, y: 1600, width: 1000, height: 800 });
        assert_eq!(w.region, Region { x: 3000, y: 1200, width: 2000, height: 1600 });
    }

    /// On a scaled output the screen has more pixels than the logical
    /// window, so a zoom that is "half size" logically can still want
    /// the file's own pixels.
    #[test]
    fn the_output_scale_counts_toward_how_sharp_a_piece_must_be() {
        assert_eq!(wanted(&at(0.5), IMAGE, VIEWPORT, 1.0, DECODED).unwrap().density, 0.5);
        assert_eq!(wanted(&at(0.5), IMAGE, VIEWPORT, 2.0, DECODED).unwrap().density, 1.0);
    }

    /// Past 1:1 there is nothing more in the file to decode.
    #[test]
    fn zooming_past_the_files_own_pixels_never_asks_for_more_of_them() {
        let w = wanted(&at(8.0), IMAGE, VIEWPORT, 1.6, DECODED).unwrap();
        assert_eq!(w.density, 1.0);
        // 125 picture pixels across, plus the one each edge rounds out to.
        assert!(w.visible.width <= 126, "{w:?}");
    }

    /// A small picture decoded whole is already as sharp as anything.
    #[test]
    fn a_picture_decoded_at_its_own_size_never_wants_a_piece() {
        assert_eq!(wanted(&at(4.0), IMAGE, VIEWPORT, 1.0, 1.0), None);
    }

    /// The margin is cut at the picture's edge, never past it.
    #[test]
    fn a_piece_at_the_edge_of_the_picture_stays_inside_it() {
        let mut t = at(1.0);
        t.pan_by(10_000.0, 10_000.0, IMAGE, VIEWPORT);
        let w = wanted(&t, IMAGE, VIEWPORT, 1.0, DECODED).unwrap();
        assert_eq!((w.region.x, w.region.y), (0, 0));
        assert!(w.region.x + w.region.width <= 8000 && w.region.y + w.region.height <= 4000);
    }

    /// A pan inside the margin is met by the piece already held; a pan
    /// past it, or a zoom in that wants it sharper, is not.
    #[test]
    fn a_piece_held_is_reused_until_the_view_leaves_it() {
        let first = wanted(&at(0.5), IMAGE, VIEWPORT, 1.0, DECODED).unwrap();
        let mut t = at(0.5);
        t.pan_by(200.0, 0.0, IMAGE, VIEWPORT);
        let panned = wanted(&t, IMAGE, VIEWPORT, 1.0, DECODED).unwrap();
        assert!(panned.met_by(first.region, first.density));
        t.pan_by(1000.0, 0.0, IMAGE, VIEWPORT);
        let far = wanted(&t, IMAGE, VIEWPORT, 1.0, DECODED).unwrap();
        assert!(!far.met_by(first.region, first.density));
        let closer = wanted(&at(1.0), IMAGE, VIEWPORT, 1.0, DECODED).unwrap();
        assert!(!closer.met_by(first.region, first.density), "sharper is wanted");
    }
}

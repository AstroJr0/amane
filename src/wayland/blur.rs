use smithay_client_toolkit::reexports::protocols::ext::background_effect::v1::client::{
    ext_background_effect_manager_v1::{self, ExtBackgroundEffectManagerV1},
    ext_background_effect_surface_v1::{self, ExtBackgroundEffectSurfaceV1},
};
use wayland_client::{Connection, Dispatch, QueueHandle};

use crate::InputArea;
use crate::graphics::BlurArea;

use super::{WaylandState, surface::OpenWindow};

// the compositor's blur behind one surface, gone before the surface is
pub struct BackgroundEffect(ExtBackgroundEffectSurfaceV1);

impl Drop for BackgroundEffect {
    fn drop(&mut self) {
        self.0.destroy();
    }
}

impl OpenWindow {
    /*
     * the compositor blurs what is behind the window, so the areas go out as a region;
     * only a changed region is sent, it takes effect with the next commit like the rest
     */
    pub fn update_blur(&mut self, areas: &[BlurArea]) {
        // a compositor without the protocol shows these rectangles as they are
        let Some(manager) = &self.effect_manager else {
            return;
        };

        let rects = blur_rects(areas, self.scale, self.width, self.height);

        if rects == self.blurred {
            return;
        }

        self.blurred = rects;

        // nothing to blur, and nothing asked for before, needs no effect object
        if self.blurred.is_empty() && self.background_effect.is_none() {
            return;
        }

        let surface = self.role.wl_surface();

        let effect = self.background_effect.get_or_insert_with(|| {
            BackgroundEffect(manager.get_background_effect(surface, &self.qh, ()))
        });

        if self.blurred.is_empty() {
            effect.0.set_blur_region(None);

            return;
        }

        let region = self.compositor.create_region(&self.qh, ());

        for rect in &self.blurred {
            region.add(rect.x, rect.y, rect.width, rect.height);
        }

        effect.0.set_blur_region(Some(&region));

        // the compositor keeps its own copy, so the region can go right away
        region.destroy();
    }
}

/*
 * a region is only made of rectangles, so a rounded corner becomes a stair of rows, one
 * per surface pixel, rows with the same ends joined; the areas are in buffer pixels
 * and the region in surface units, cut to the window since the compositor does the same
 */
fn blur_rects(areas: &[BlurArea], scale: f32, width: u32, height: u32) -> Vec<InputArea> {
    let mut rects = Vec::new();

    for blurred in areas {
        add_area(&mut rects, blurred, scale, width as i32, height as i32);
    }

    rects
}

fn add_area(rects: &mut Vec<InputArea>, blurred: &BlurArea, scale: f32, width: i32, height: i32) {
    let area = blurred.area;

    let left = area.x / scale;
    let top = area.y / scale;
    let right = (area.x + area.width) / scale;
    let bottom = (area.y + area.height) / scale;

    // a corner can't curve past the middle of the shorter side
    let reach = f32::max((right - left).min(bottom - top) / 2.0, 0.0);

    let [top_left, top_right, bottom_right, bottom_left] = blurred
        .radius
        .to_array()
        .map(|corner| (corner / scale).min(reach));

    let first = (top.floor() as i32).max(0);
    let last = (bottom.ceil() as i32).min(height);

    // the row being built, joined with the next while their ends match
    let mut open: Option<InputArea> = None;

    for row in first..last {
        let middle = row as f32 + 0.5;

        let from_top = middle - top;
        let from_bottom = bottom - middle;

        let inset_left = f32::max(inset(top_left, from_top), inset(bottom_left, from_bottom));
        let inset_right = f32::max(inset(top_right, from_top), inset(bottom_right, from_bottom));

        // a straight edge keeps the whole pixel it touches, a curve only what is inside
        let start = if inset_left > 0.0 {
            (left + inset_left).ceil() as i32
        } else {
            left.floor() as i32
        };

        let end = if inset_right > 0.0 {
            (right - inset_right).floor() as i32
        } else {
            right.ceil() as i32
        };

        let start = start.max(0);
        let end = end.min(width);

        if end <= start {
            continue;
        }

        let joins = open
            .as_ref()
            .is_some_and(|rect| rect.x == start && rect.x + rect.width == end);

        if joins {
            if let Some(rect) = &mut open {
                rect.height += 1;
            }

            continue;
        }

        rects.extend(open.take());

        open = Some(InputArea {
            x: start,
            y: row,
            width: end - start,
            height: 1,
        });
    }

    rects.extend(open);
}

// how far a corner of this radius cuts into a row whose middle is this far from the edge
fn inset(radius: f32, depth: f32) -> f32 {
    if radius <= 0.0 || depth >= radius {
        return 0.0;
    }

    let rise = radius - depth.max(0.0);

    radius - f32::sqrt(f32::max(radius * radius - rise * rise, 0.0))
}

// the manager only says which effects it can do, and the compositor drops the ones it can't
impl Dispatch<ExtBackgroundEffectManagerV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ExtBackgroundEffectManagerV1,
        _: ext_background_effect_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ExtBackgroundEffectSurfaceV1,
        _: ext_background_effect_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::{Area, Corners};

    fn plain(area: Area) -> BlurArea {
        BlurArea {
            area,
            radius: Corners::default(),
        }
    }

    fn covered(rects: &[InputArea]) -> i32 {
        rects.iter().map(|rect| rect.width * rect.height).sum()
    }

    #[test]
    fn a_square_rectangle_is_one_rectangle() {
        let rects = blur_rects(&[plain(Area::new(10.0, 20.0, 100.0, 50.0))], 1.0, 400, 300);

        assert_eq!(
            rects,
            [InputArea {
                x: 10,
                y: 20,
                width: 100,
                height: 50
            }],
        );
    }

    #[test]
    fn buffer_pixels_become_surface_units() {
        let rects = blur_rects(&[plain(Area::new(20.0, 40.0, 200.0, 100.0))], 2.0, 400, 300);

        assert_eq!(
            rects,
            [InputArea {
                x: 10,
                y: 20,
                width: 100,
                height: 50
            }],
        );
    }

    #[test]
    fn rounded_corners_cut_in_symmetrically_and_cover_less() {
        let area = BlurArea {
            area: Area::new(0.0, 0.0, 100.0, 100.0),
            radius: Corners::from(20.0),
        };

        let rects = blur_rects(&[area], 1.0, 200, 200);

        assert!(rects.len() > 3);
        assert!(covered(&rects) < 100 * 100);

        // the top row is narrower than the middle, and the shape mirrors left to right
        assert!(rects[0].width < 100);

        for rect in &rects {
            assert_eq!(rect.x + rect.width, 100 - rect.x);
        }

        // and top to bottom
        let first = rects.first().unwrap();
        let last = rects.last().unwrap();

        assert_eq!(first.width, last.width);
        assert_eq!(first.y, 100 - (last.y + last.height));
    }

    #[test]
    fn the_region_stays_inside_the_window() {
        let rects = blur_rects(
            &[plain(Area::new(-50.0, -50.0, 1000.0, 1000.0))],
            1.0,
            80,
            60,
        );

        assert_eq!(
            rects,
            [InputArea {
                x: 0,
                y: 0,
                width: 80,
                height: 60
            }],
        );
    }

    #[test]
    fn a_pill_has_no_straight_middle_but_still_closes() {
        let area = BlurArea {
            area: Area::new(0.0, 0.0, 40.0, 40.0),
            radius: Corners::from(1000.0),
        };

        let rects = blur_rects(&[area], 1.0, 100, 100);

        // a circle of radius 20 covers about pi * 20 * 20
        let pixels = covered(&rects);

        assert!((1100..1400).contains(&pixels), "covered {pixels}");
    }
}    

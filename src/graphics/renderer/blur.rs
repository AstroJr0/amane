use crate::graphics::{Area, Corners, Transform};

use super::Renderer;

// a rounded rectangle whose background the compositor blurs, in buffer pixels
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurArea {
    pub area: Area,
    pub radius: Corners,
}

impl Renderer {
    // nothing is drawn, the area is handed to the compositor with the frame
    pub fn blur(&mut self, area: Area, radius: Corners) {
        if area.width <= 0.0 || area.height <= 0.0 {
            return;
        }

        self.blurs.push(device_shape(self.transform, area, radius));
    }

    pub fn take_blurs(&mut self) -> Vec<BlurArea> {
        std::mem::take(&mut self.blurs)
    }
}

// where a rounded rectangle lands in the buffer; a turned or stretched one is covered by its bounding box
pub(super) fn device_shape(transform: Transform, area: Area, radius: Corners) -> BlurArea {
    if let Some(scale) = transform.even_scale() {
        let (x, y) = transform.map(area.x, area.y);

        return BlurArea {
            area: Area::new(x, y, area.width * scale, area.height * scale),
            radius: radius.map(|corner| corner * scale),
        };
    }

    let right = area.x + area.width;
    let bottom = area.y + area.height;

    let points = [
        (area.x, area.y),
        (right, area.y),
        (right, bottom),
        (area.x, bottom),
    ]
    .map(|(x, y)| transform.map(x, y));

    let left = points
        .iter()
        .map(|point| point.0)
        .fold(f32::INFINITY, f32::min);
    let top = points
        .iter()
        .map(|point| point.1)
        .fold(f32::INFINITY, f32::min);
    let far = points
        .iter()
        .map(|point| point.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let low = points
        .iter()
        .map(|point| point.1)
        .fold(f32::NEG_INFINITY, f32::max);

    BlurArea {
        area: Area::new(left, top, far - left, low - top),
        radius: Corners::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scaled_rectangle_keeps_its_corners() {
        let mut renderer = Renderer::new(2.0);

        renderer.blur(Area::new(5.0, 10.0, 20.0, 30.0), Corners::from(4.0));

        assert_eq!(
            renderer.take_blurs(),
            [BlurArea {
                area: Area::new(10.0, 20.0, 40.0, 60.0),
                radius: Corners::from(8.0),
            }],
        );
        assert!(renderer.take_blurs().is_empty());
    }

    #[test]
    fn a_turned_rectangle_is_covered_by_its_bounding_box() {
        let turned = Transform::from_rotate(90.0);

        let shape = device_shape(turned, Area::new(0.0, 0.0, 10.0, 20.0), Corners::from(3.0));

        assert_eq!(shape.radius, Corners::default());
        assert!((shape.area.x + 20.0).abs() < 0.001);
        assert!(shape.area.y.abs() < 0.001);
        assert!((shape.area.width - 20.0).abs() < 0.001);
        assert!((shape.area.height - 10.0).abs() < 0.001);
    }

    #[test]
    fn an_empty_rectangle_blurs_nothing() {
        let mut renderer = Renderer::new(1.0);

        renderer.blur(Area::new(0.0, 0.0, 0.0, 10.0), Corners::default());

        assert!(renderer.take_blurs().is_empty());
    }
}
                                

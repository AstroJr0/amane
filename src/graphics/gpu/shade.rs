use std::num::NonZeroU64;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Instant;

use vello::wgpu::util::{BufferInitDescriptor, DeviceExt};
use vello::wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindingResource, BufferBinding, BufferUsages, LoadOp,
    Operations, RenderPassColorAttachment, RenderPassDescriptor, StoreOp, Texture,
};

use crate::graphics::{Area, Corners, Transform, VALUE_ROWS};

use super::paint::device_mask;
use super::shader::Shader;
use super::{Gpu, texture};

// every shader's time counts from when the first one was drawn
static START: LazyLock<Instant> = LazyLock::new(Instant::now);

/*
 * time starts over after this many seconds: a shell can run for days, and
 * an f32 of that many seconds is too coarse to move an animation smoothly;
 * motion that repeats every few seconds, like sin(time * 2.0), only skips
 * when the repeat doesn't divide this evenly
 */
const TIME_PERIOD: f64 = 3600.0;

// the inputs sit in one buffer, one slot each: size, time, placement and values, as the bindings 0 to 3
const SLOT: usize = 256;

const SLOTS: usize = 4;

// how many bytes of each slot the shader reads
const USED: [u64; SLOTS] = [16, 16, 64, (VALUE_ROWS * 16) as u64];

impl Gpu {
    /*
     * with rounded corners the shader draws on a canvas of its own, which is
     * trimmed to the rounded corners and laid over what the commands before it drew;
     * with square ones it blends straight onto the canvas, saving three full
     * size passes and a texture every frame
     */
    pub(super) fn shade(
        &mut self,
        canvas: &Texture,
        shader: &Path,
        values: &[[f32; 4]],
        area: Area,
        radius: Corners,
        transform: Transform,
    ) {
        // a rectangle scrolled out of view would still cost every pass below
        if !on_canvas(area, transform, canvas.width(), canvas.height()) {
            return;
        }

        let Some(shader) = self.shader(shader) else {
            return;
        };

        // the shader only covers the area, so square corners need no trimming
        if radius.largest() <= 0.0 {
            self.draw_shader(canvas, &shader, values, area, transform);

            return;
        }

        let layer = self.take_canvas(canvas.width(), canvas.height());

        self.draw_shader(&layer, &shader, values, area, transform);

        self.trim_corners(&layer, area, radius, transform);

        self.lay(&layer, canvas, 1.0, false);

        self.give_back(layer);
    }

    /*
     * erases what lies outside the rounded rectangle; one quad does it unless the
     * shape is turned, stretched unevenly or under a pixel, which only vello can trace
     */
    fn trim_corners(&mut self, layer: &Texture, area: Area, radius: Corners, transform: Transform) {
        if let Some(shape) = device_mask(area, radius, transform) {
            let mut encoder = self.take_encoder();

            self.quads.trim(&self.device, &mut encoder, layer, shape);

            self.keep_encoder(encoder);
        } else if let Some(path) = area.trace(radius) {
            self.trim(layer, &path, transform);
        }
    }

    // compiled the first time any window draws it, then kept; None when it can't be used
    fn shader(&self, path: &Path) -> Option<Shader> {
        let mut shaders = self.shaders.borrow_mut();

        if let Some(known) = shaders.get(path) {
            return known.clone();
        }

        let loaded = match Shader::load(&self.device, path) {
            Ok(shader) => Some(shader),

            // said once, the shader then draws nothing instead of failing every frame
            Err(error) => {
                eprintln!("amane: shader {}: {error}", path.display());

                None
            }
        };

        shaders.insert(path.to_path_buf(), loaded.clone());

        loaded
    }

    fn draw_shader(
        &mut self,
        layer: &Texture,
        shader: &Shader,
        values: &[[f32; 4]],
        area: Area,
        transform: Transform,
    ) {
        let time = (START.elapsed().as_secs_f64() % TIME_PERIOD) as f32;

        // the shader always reads every row of values, so the ones left out stay 0
        let mut bytes = [0; SLOT * SLOTS];

        write(&mut bytes, 0, &[[area.width, area.height, 0.0, 0.0]]);

        write(&mut bytes, 1, &[[time, 0.0, 0.0, 0.0]]);

        write(
            &mut bytes,
            2,
            &[
                [area.x, area.y, area.width, area.height],
                [transform.sx, transform.kx, transform.tx, 0.0],
                [transform.ky, transform.sy, transform.ty, 0.0],
                [layer.width() as f32, layer.height() as f32, 0.0, 0.0],
            ],
        );

        write(&mut bytes, 3, values);

        let buffer = self.device.create_buffer_init(&BufferInitDescriptor {
            label: None,
            contents: &bytes,
            usage: BufferUsages::UNIFORM,
        });

        let entries: [BindGroupEntry; SLOTS] = std::array::from_fn(|slot| BindGroupEntry {
            binding: slot as u32,
            resource: BindingResource::Buffer(BufferBinding {
                buffer: &buffer,
                offset: (slot * SLOT) as u64,
                size: NonZeroU64::new(USED[slot]),
            }),
        });

        let inputs = self.device.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &shader.inputs,
            entries: &entries,
        });

        let target = texture::view(layer);

        let attachment = RenderPassColorAttachment {
            view: &target,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
        };

        let mut encoder = self.take_encoder();

        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                color_attachments: &[Some(attachment)],
                ..Default::default()
            });

            pass.set_pipeline(&shader.pipeline);

            pass.set_bind_group(0, &inputs, &[]);

            // the six corners of the rectangle's two triangles
            pass.draw(0..6, 0..1);
        }

        self.keep_encoder(encoder);
    }
}

// the rows' numbers go to the start of the slot, whatever they don't fill stays as it was
fn write(bytes: &mut [u8], slot: usize, rows: &[[f32; 4]]) {
    let slot = &mut bytes[slot * SLOT..(slot + 1) * SLOT];

    let numbers = rows.iter().flatten();

    let (targets, _) = slot.as_chunks_mut::<4>();

    for (target, number) in targets.iter_mut().zip(numbers) {
        *target = number.to_ne_bytes();
    }
}

// false when the rectangle, once transformed, lies wholly outside the canvas
fn on_canvas(area: Area, transform: Transform, width: u32, height: u32) -> bool {
    let corners = [
        (area.x, area.y),
        (area.x + area.width, area.y),
        (area.x, area.y + area.height),
        (area.x + area.width, area.y + area.height),
    ];

    let mut left = f32::INFINITY;
    let mut top = f32::INFINITY;
    let mut right = f32::NEG_INFINITY;
    let mut bottom = f32::NEG_INFINITY;

    for (x, y) in corners {
        let x_device = transform.sx * x + transform.kx * y + transform.tx;
        let y_device = transform.ky * x + transform.sy * y + transform.ty;

        left = left.min(x_device);
        right = right.max(x_device);
        top = top.min(y_device);
        bottom = bottom.max(y_device);
    }

    right > 0.0 && bottom > 0.0 && left < width as f32 && top < height as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_rectangles_wholly_off_the_canvas() {
        let area = Area::new(0.0, 0.0, 100.0, 50.0);

        assert!(on_canvas(area, Transform::IDENTITY, 200, 200));
        assert!(on_canvas(area, Transform::from_rotate(45.0), 200, 200));
        assert!(on_canvas(area, Transform::from_scale(2.0, 2.0), 150, 80));

        // scrolled up past the top, and past the right edge
        assert!(!on_canvas(
            Area::new(0.0, -80.0, 100.0, 50.0),
            Transform::IDENTITY,
            200,
            200
        ));
        assert!(!on_canvas(
            Area::new(200.0, 0.0, 100.0, 50.0),
            Transform::IDENTITY,
            200,
            200
        ));
    }

    #[test]
    fn keeps_partly_visible_rectangles() {
        let area = Area::new(-50.0, -25.0, 100.0, 50.0);

        assert!(on_canvas(area, Transform::IDENTITY, 200, 200));
    }

    #[test]
    fn writes_rows_to_the_start_of_their_slot() {
        let mut bytes = [0; SLOT * SLOTS];

        write(&mut bytes, 2, &[[1.0, 2.0, 3.0, 4.0], [5.0, 0.0, 0.0, 0.0]]);

        let at = |slot: usize, index: usize| {
            let start = slot * SLOT + index * 4;

            f32::from_ne_bytes(bytes[start..start + 4].try_into().unwrap())
        };

        assert_eq!(at(2, 0), 1.0);
        assert_eq!(at(2, 3), 4.0);
        assert_eq!(at(2, 4), 5.0);
        assert_eq!(at(1, 0), 0.0);
        assert_eq!(at(3, 0), 0.0);
    }

    #[test]
    fn writing_more_rows_than_fit_stays_inside_the_slot() {
        let mut bytes = [0; SLOT * SLOTS];

        write(&mut bytes, 2, &[[9.0; 4]; 32]);

        assert_eq!(&bytes[3 * SLOT..], &[0; SLOT][..]);
    }
}

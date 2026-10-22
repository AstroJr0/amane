use vello::wgpu::util::{BufferInitDescriptor, DeviceExt};
use vello::wgpu::{
    BindGroupDescriptor, BindGroupEntry, BindingResource, BufferUsages, CommandEncoder, Device,
    LoadOp, Operations, RenderPassColorAttachment, RenderPassDescriptor, StoreOp, Texture,
};

use crate::graphics::gpu::texture;

use super::{Clip, Quads};

impl Quads {
    // before any letter of the frame is placed
    pub fn start_frame(&mut self) {
        self.atlas.start_frame();
    }

    // draws everything gathered onto the canvas, over what is already there
    pub fn draw(&mut self, device: &Device, encoder: &mut CommandEncoder, canvas: &Texture) {
        self.draw_with(device, encoder, canvas, false);
    }

    pub fn erase(
        &mut self,
        device: &Device,
        encoder: &mut CommandEncoder,
        canvas: &Texture,
        shape: Clip,
        strength: f32,
    ) {
        self.mask(shape, strength);

        self.draw_with(device, encoder, canvas, true);
    }

    fn draw_with(
        &mut self,
        device: &Device,
        encoder: &mut CommandEncoder,
        canvas: &Texture,
        erase: bool,
    ) {
        if self.waiting.is_empty() {
            return;
        }

        let numbers: Vec<u8> = self
            .waiting
            .drain(..)
            .flat_map(|value| value.to_ne_bytes())
            .collect();

        let quads = device.create_buffer_init(&BufferInitDescriptor {
            label: None,
            contents: &numbers,
            usage: BufferUsages::STORAGE,
        });

        let size: Vec<u8> = [canvas.width() as f32, canvas.height() as f32, 0.0, 0.0]
            .iter()
            .flat_map(|value| value.to_ne_bytes())
            .collect();

        let size = device.create_buffer_init(&BufferInitDescriptor {
            label: None,
            contents: &size,
            usage: BufferUsages::UNIFORM,
        });

        let atlas = texture::view(self.atlas.texture());

        let inputs = device.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &self.inputs,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: quads.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: size.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&atlas),
                },
            ],
        });

        let target = texture::view(canvas);

        {
            let attachment = RenderPassColorAttachment {
                view: &target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            };

            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                color_attachments: &[Some(attachment)],
                ..Default::default()
            });

            pass.set_pipeline(if erase {
                &self.erase_pipeline
            } else {
                &self.pipeline
            });

            pass.set_bind_group(0, &inputs, &[]);

            for run in self.runs.drain(..) {
                let picture = run
                    .picture
                    .and_then(|key| self.pictures.get(&key))
                    .unwrap_or(&self.blank);

                pass.set_bind_group(1, picture, &[]);

                // six corners make each quad's two triangles
                pass.draw(0..6, run.first..run.first + run.count);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use vello::wgpu::{
        BufferDescriptor, BufferUsages, DeviceDescriptor, Extent3d, Instance, InstanceDescriptor,
        MapMode, PollType, RequestAdapterOptions, TexelCopyBufferInfo, TexelCopyBufferLayout,
    };

    use crate::graphics::gpu::wait::wait;
    use crate::graphics::{Area, Color, Corners};

    use super::super::Clips;
    use super::*;

    #[test]
    #[ignore = "requires a graphics adapter"]
    fn erases_rounded_masks_without_changing_later_drawing() {
        let instance = Instance::new(InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = wait(instance.request_adapter(&RequestAdapterOptions::default()))
            .expect("failed to find a gpu");
        let (device, queue) =
            wait(adapter.request_device(&DeviceDescriptor::default())).expect("failed to open gpu");
        let canvas = texture::canvas(&device, 16, 16);
        let mut quads = Quads::new(&device, &queue);

        // every pass shares one encoder, like a real frame
        let mut encoder = device.create_command_encoder(&Default::default());

        quads.rectangle(
            Area::new(0.0, 0.0, 16.0, 16.0),
            Corners::default(),
            Color::BLUE,
            Clips::default(),
        );
        quads.draw(&device, &mut encoder, &canvas);
        quads.erase(
            &device,
            &mut encoder,
            &canvas,
            Clip {
                area: Area::new(4.0, 4.0, 8.0, 8.0),
                radius: Corners::from(4.0),
            },
            1.0,
        );
        quads.erase(
            &device,
            &mut encoder,
            &canvas,
            Clip {
                area: Area::new(0.0, 0.0, 4.0, 4.0),
                radius: Corners::default(),
            },
            0.25,
        );
        quads.rectangle(
            Area::new(0.0, 12.0, 16.0, 4.0),
            Corners::default(),
            Color::RED,
            Clips::default(),
        );
        quads.draw(&device, &mut encoder, &canvas);

        let readback = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 256 * 16,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            canvas.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(16),
                },
            },
            Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let (send, receive) = mpsc::channel();
        readback.slice(..).map_async(MapMode::Read, move |result| {
            send.send(result).unwrap();
        });
        device.poll(PollType::wait_indefinitely()).unwrap();
        receive.recv().unwrap().unwrap();

        let pixels = readback.slice(..).get_mapped_range().unwrap();
        let pixel = |x: usize, y: usize| &pixels[y * 256 + x * 4..y * 256 + x * 4 + 4];
        assert_eq!(pixel(8, 8), [0, 0, 0, 0]);
        assert_eq!(pixel(4, 4), [0, 0, 255, 255]);
        assert_eq!(pixel(1, 1), [0, 0, 191, 191]);
        assert_eq!(pixel(8, 14), [255, 0, 0, 255]);
    }
}

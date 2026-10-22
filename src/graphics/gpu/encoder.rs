use vello::wgpu::CommandEncoder;

use super::Gpu;

impl Gpu {
    // the frame's encoder so far, or a new one for the frame's first job
    pub(super) fn take_encoder(&self) -> CommandEncoder {
        match self.pending.take() {
            Some(encoder) => encoder,
            None => self.device.create_command_encoder(&Default::default()),
        }
    }

    // more jobs can still follow, so nothing is mailed yet
    pub(super) fn keep_encoder(&self, encoder: CommandEncoder) {
        self.pending.replace(Some(encoder));
    }

    /*
     * mails what was written so far; vello mails its own list, so this
     * runs before vello, or vello would draw before our earlier jobs
     */
    pub(super) fn submit(&self) {
        let Some(encoder) = self.pending.take() else {
            return;
        };

        self.queue.submit([encoder.finish()]);
    }
}

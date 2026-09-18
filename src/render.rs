//! Dock scene adaptation over the same admitted GPU/readback owner used by Lom.
//! Construction is explicit; ordinary configuration and view tests never call it.
use crate::ui::DockScene;
pub use sophia_shell_gpu::{GpuAdmissionEvidence, GpuGrant};
mod worker;
pub use worker::{GpuWorker, RenderJobId, RenderResult};

/// Sequential GPU rasterizer. A protocol worker must own this object and its
/// retained hosts; this adapter does not create a second application loop.
pub struct DockRenderer(sophia_shell_gpu::GpuPreview);
impl DockRenderer {
    /// Initialize against the explicit exact-device grant. No CPU fallback.
    pub fn new(grant: &GpuGrant) -> Result<(Self, GpuAdmissionEvidence), String> {
        let renderer = sophia_shell_gpu::GpuPreview::new(grant)?;
        let evidence = renderer.evidence(grant)?;
        Ok((Self(renderer), evidence))
    }
    /// Render the retained host's scene to packed RGBA. Readback is bounded;
    /// success does not mean upload, Prepared or native Presented occurred.
    pub fn render(&mut self, scene: &mut DockScene) -> Result<Vec<u8>, String> {
        self.0
            .read_rgba(&mut scene.scene, scene.width, scene.height)
    }
}

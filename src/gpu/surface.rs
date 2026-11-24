//! Surfaces: the images a window shows, drawn into as targets and presented.
//!
//! A surface is made on the device's own instance from any window handle, so nothing here knows
//! which window system made the window. Each frame takes the surface's next image as a jess
//! [`Texture`], which a pass or a plan draws into like any target, and presents it.

use super::texture::{TextureDesc, TextureShape, TextureUsage};
use super::{Gpu, GpuError, Texture};
use crate::render::TextureFormat;

/// When a presented image reaches the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PresentMode {
    /// At the display's next refresh, one image a refresh: no tearing, and presenting waits.
    #[default]
    Vsync,
    /// As soon as it can: tearing allowed, and presenting never waits for the display.
    Immediate,
}

/// The formats a surface may draw in, the plain ones first, which is the order one is chosen in
/// when the caller names none: a palette or a texture copied to the screen is already encoded.
const FORMATS: [TextureFormat; 4] = [
    TextureFormat::Bgra8Unorm,
    TextureFormat::Rgba8Unorm,
    TextureFormat::Bgra8UnormSrgb,
    TextureFormat::Rgba8UnormSrgb,
];

/// A window's surface on a device, configured for its size.
pub struct Surface {
    /// None between losing the surface and making it again, which is tried every frame until it
    /// works.
    raw: Option<wgpu::Surface<'static>>,
    config: wgpu::SurfaceConfiguration,
    format: TextureFormat,
    /// The present modes the surface offers, as it said when it was made.
    modes: Vec<wgpu::PresentMode>,
    /// The window again, for a surface made anew when the old one is lost.
    window: Box<dyn Fn() -> wgpu::SurfaceTarget<'static> + Send>,
}

impl std::fmt::Debug for Surface {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Surface")
            .field("format", &self.format)
            .field("width", &self.config.width)
            .field("height", &self.config.height)
            .finish_non_exhaustive()
    }
}

/// One image of a surface, drawn into and then presented.
#[derive(Debug)]
pub struct SurfaceFrame {
    raw: wgpu::SurfaceTexture,
    texture: Texture,
}

impl SurfaceFrame {
    /// The image as a target: a 2D texture of the surface's format and size.
    #[must_use]
    pub fn texture(&self) -> &Texture {
        &self.texture
    }
}

impl Gpu {
    /// A surface on `window`, `width` by `height` pixels, in `format` or, without one, the first
    /// of the plain 8-bit formats the surface offers. The surface keeps a handle on `window`, to
    /// make itself again on it if it is lost.
    ///
    /// # Errors
    ///
    /// A window the device's instance can't make a surface on, an adapter that can't present to
    /// it, or a format it doesn't offer.
    pub fn surface<W>(
        &self,
        window: W,
        width: u32,
        height: u32,
        format: Option<TextureFormat>,
        mode: PresentMode,
    ) -> Result<Surface, GpuError>
    where
        W: Into<wgpu::SurfaceTarget<'static>> + Clone + Send + 'static,
    {
        let raw = self.raw_surface(window.clone().into())?;
        let capabilities = raw.get_capabilities(&self.adapter);
        if capabilities.formats.is_empty() {
            return Err(GpuError::Device("the adapter can't present to this window".to_owned()));
        }
        let offered: Vec<TextureFormat> = TextureFormat::ALL
            .into_iter()
            .filter(|format| capabilities.formats.contains(&super::texture::raw_format(*format)))
            .collect();
        let chosen = match format {
            Some(format) if offered.contains(&format) => format,
            Some(format) => {
                return Err(GpuError::Invalid(format!(
                    "the window's surface doesn't draw in {format}: it offers {offered:?}"
                )));
            }
            None => FORMATS
                .into_iter()
                .find(|format| offered.contains(format))
                .ok_or_else(|| GpuError::Invalid(format!("the window's surface offers only {offered:?}")))?,
        };
        let present_mode = concrete(&capabilities.present_modes, mode);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | (capabilities.usages & wgpu::TextureUsages::COPY_SRC),
            format: super::texture::raw_format(chosen),
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            desired_maximum_frame_latency: 2,
            present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: Vec::new(),
        };
        self.checked(|| raw.configure(&self.device, &config))?;
        Ok(Surface {
            raw: Some(raw),
            config,
            format: chosen,
            modes: capabilities.present_modes,
            window: Box::new(move || window.clone().into()),
        })
    }

    fn raw_surface(&self, window: wgpu::SurfaceTarget<'static>) -> Result<wgpu::Surface<'static>, GpuError> {
        self.instance
            .create_surface(window)
            .map_err(|problem| GpuError::Device(format!("no surface on the window: {problem}")))
    }

    /// Presents `frame`: what was drawn into it reaches the window.
    pub fn present(&self, frame: SurfaceFrame) {
        self.queue.present(frame.raw);
    }
}

impl Surface {
    /// The format its images are in.
    #[must_use]
    pub fn format(&self) -> TextureFormat {
        self.format
    }

    /// Whether presenting waits for the display: what its present mode in effect does.
    #[must_use]
    pub fn vsync(&self) -> bool {
        waits(self.config.present_mode)
    }

    /// Whether presenting would wait for the display if it were asked for `mode`: a surface that
    /// offers no mode that doesn't wait (every surface offers one that does) waits whatever is
    /// asked.
    #[must_use]
    pub fn waits_for(&self, mode: PresentMode) -> bool {
        waits(concrete(&self.modes, mode))
    }

    /// Configures it to present as `mode` asks, as far as it can: see [`Surface::waits_for`].
    /// Call it only with none of its images taken; the presenter does, before it takes the next.
    ///
    /// # Errors
    ///
    /// What the device refuses.
    pub fn set_present(&mut self, gpu: &Gpu, mode: PresentMode) -> Result<(), GpuError> {
        let present_mode = concrete(&self.modes, mode);
        if present_mode == self.config.present_mode {
            return Ok(());
        }
        self.config.present_mode = present_mode;
        let Some(raw) = &self.raw else {
            // Made again in this mode when it next draws.
            return Ok(());
        };
        gpu.checked(|| raw.configure(&gpu.device, &self.config))
    }

    /// Its size in pixels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Sizes it again, for a window `width` by `height` pixels. A window shrunk to nothing keeps
    /// a surface of one pixel.
    ///
    /// # Errors
    ///
    /// What the device refuses.
    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) -> Result<(), GpuError> {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        let Some(raw) = &self.raw else {
            // Made again at this size when it next draws.
            return Ok(());
        };
        gpu.checked(|| raw.configure(&gpu.device, &self.config))
    }

    /// Its next image, or `None` when there is none to draw this frame: the window is hidden or
    /// covered, the wait timed out, or the surface changed under it, in which case it is
    /// configured again for the next. A surface that was lost (the display changed, the driver
    /// restarted) is made again on its window, and this frame has no image either.
    ///
    /// # Errors
    ///
    /// A lost surface that can't be made again, or one the device refuses.
    pub fn frame(&mut self, gpu: &Gpu) -> Result<Option<SurfaceFrame>, GpuError> {
        let Some(surface) = &self.raw else {
            self.remake(gpu)?;
            return Ok(None);
        };
        let raw = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(raw) | wgpu::CurrentSurfaceTexture::Suboptimal(raw) => raw,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return Ok(None),
            wgpu::CurrentSurfaceTexture::Outdated => {
                gpu.checked(|| surface.configure(&gpu.device, &self.config))?;
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.remake(gpu)?;
                return Ok(None);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(GpuError::Backend("the window's surface refused its next image".to_owned()));
            }
        };
        let desc = TextureDesc {
            width: self.config.width,
            height: self.config.height,
            shape: TextureShape::D2,
            format: self.format,
            mips: 1,
            samples: 1,
            usage: TextureUsage::TARGET,
        };
        let texture = Texture::wrap(gpu, raw.texture.clone(), desc);
        Ok(Some(SurfaceFrame { raw, texture }))
    }

    /// Drops the surface as a lost one is, for a test of what comes after.
    #[cfg(test)]
    pub(crate) fn lose(&mut self) {
        self.raw = None;
    }

    /// Makes the surface again on its window, configured as it was. The lost one is dropped
    /// first, since a window may have only one; if the new one can't be made or configured, there
    /// is none until the next try.
    fn remake(&mut self, gpu: &Gpu) -> Result<(), GpuError> {
        self.raw = None;
        let lost =
            |problem: GpuError| GpuError::Device(format!("the window's surface was lost and isn't back yet: {problem}"));
        let raw = gpu.raw_surface((self.window)()).map_err(lost)?;
        gpu.checked(|| raw.configure(&gpu.device, &self.config)).map_err(lost)?;
        self.raw = Some(raw);
        Ok(())
    }
}

/// The mode a surface offering `offered` presents in when asked for `mode`: one that waits for the
/// display, preferring one that doesn't hold a late frame back a whole refresh; or one that doesn't
/// wait, preferring one that may tear to one that drops frames, and one that waits if it offers
/// neither.
fn concrete(offered: &[wgpu::PresentMode], mode: PresentMode) -> wgpu::PresentMode {
    let preferred: &[wgpu::PresentMode] = match mode {
        PresentMode::Vsync => &[wgpu::PresentMode::FifoRelaxed],
        PresentMode::Immediate => &[wgpu::PresentMode::Immediate, wgpu::PresentMode::Mailbox],
    };
    preferred
        .iter()
        .copied()
        .find(|preferred| offered.contains(preferred))
        .unwrap_or(wgpu::PresentMode::Fifo)
}

/// Whether presenting in `mode` waits for the display.
fn waits(mode: wgpu::PresentMode) -> bool {
    matches!(mode, wgpu::PresentMode::Fifo | wgpu::PresentMode::FifoRelaxed)
}

#[cfg(test)]
mod tests {
    use super::{PresentMode, concrete, waits};

    /// Asking a surface not to wait for its display is answered truthfully: a surface that offers
    /// only the mode every surface must still waits, and says so.
    #[test]
    fn a_present_mode_resolves_to_what_the_surface_offers() {
        use wgpu::PresentMode::{Fifo, FifoRelaxed, Immediate, Mailbox};
        assert_eq!(concrete(&[Fifo], PresentMode::Immediate), Fifo);
        assert!(waits(concrete(&[Fifo], PresentMode::Immediate)));
        assert_eq!(concrete(&[Fifo, Mailbox], PresentMode::Immediate), Mailbox);
        assert_eq!(concrete(&[Fifo, Mailbox, Immediate], PresentMode::Immediate), Immediate);
        assert!(!waits(Mailbox) && !waits(Immediate));
        assert_eq!(concrete(&[Fifo, Immediate], PresentMode::Vsync), Fifo);
        assert_eq!(concrete(&[Fifo, FifoRelaxed], PresentMode::Vsync), FifoRelaxed);
        assert!(waits(FifoRelaxed));
    }
}

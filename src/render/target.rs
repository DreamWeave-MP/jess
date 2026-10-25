//! Target formats and sizes.

use std::fmt;

/// The pixel format of a texture: a render or depth target, or one a shader reads.
///
/// Jess owns this list. A backend maps each variant to its own format type; no variant names one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TextureFormat {
    /// Four 8-bit unsigned normalized channels.
    Rgba8Unorm,
    /// Four 8-bit unsigned normalized channels, sRGB-encoded.
    Rgba8UnormSrgb,
    /// Four 8-bit unsigned normalized channels in BGRA order, the usual window surface format.
    Bgra8Unorm,
    /// Four 8-bit unsigned normalized channels in BGRA order, sRGB-encoded.
    Bgra8UnormSrgb,
    /// Four 16-bit float channels, for HDR scene color.
    Rgba16Float,
    /// One 8-bit unsigned normalized channel, for masks.
    R8Unorm,
    /// One 32-bit float channel.
    R32Float,
    /// One 32-bit unsigned integer channel, for picking ids.
    R32Uint,
    /// Depth with at least 24 bits of precision; the backend picks the layout.
    Depth24Plus,
    /// 32-bit float depth.
    Depth32Float,
}

impl TextureFormat {
    /// Whether this is a depth format, which only a depth target may have.
    #[must_use]
    pub const fn is_depth(self) -> bool {
        matches!(self, Self::Depth24Plus | Self::Depth32Float)
    }

    /// Whether this is an unsigned integer color format, cleared with [`crate::render::ClearValue::Uint`].
    #[must_use]
    pub const fn is_integer(self) -> bool {
        matches!(self, Self::R32Uint)
    }

    /// The format's name, as the debug dump prints it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rgba8Unorm => "Rgba8Unorm",
            Self::Rgba8UnormSrgb => "Rgba8UnormSrgb",
            Self::Bgra8Unorm => "Bgra8Unorm",
            Self::Bgra8UnormSrgb => "Bgra8UnormSrgb",
            Self::Rgba16Float => "Rgba16Float",
            Self::R8Unorm => "R8Unorm",
            Self::R32Float => "R32Float",
            Self::R32Uint => "R32Uint",
            Self::Depth24Plus => "Depth24Plus",
            Self::Depth32Float => "Depth32Float",
        }
    }
}

impl fmt::Display for TextureFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(self.name())
    }
}

/// A width and height in pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Extent {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// How big a target is: fixed, or a multiple of the viewport it is rendered for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TargetSize {
    /// A fixed size in pixels. Both dimensions must be nonzero.
    Absolute {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// The viewport's size times `scale`, which must be finite and positive.
    Relative {
        /// The factor applied to both viewport dimensions.
        scale: f32,
    },
}

impl TargetSize {
    /// Exactly the viewport's size.
    pub const VIEWPORT: Self = Self::Relative { scale: 1.0 };

    /// Whether the size can describe a real target: nonzero pixels, or a finite positive scale.
    #[must_use]
    pub fn is_valid(self) -> bool {
        match self {
            Self::Absolute { width, height } => width > 0 && height > 0,
            Self::Relative { scale } => scale.is_finite() && scale > 0.0,
        }
    }

    /// The size in pixels for a viewport of `viewport` pixels.
    ///
    /// Relative sizes round to the nearest pixel and never go below one, so a minimized window
    /// still resolves to a target a backend can create.
    #[must_use]
    pub fn resolve(self, viewport: Extent) -> Extent {
        match self {
            Self::Absolute { width, height } => Extent { width, height },
            Self::Relative { scale } => Extent {
                width: scale_dimension(viewport.width, scale),
                height: scale_dimension(viewport.height, scale),
            },
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn scale_dimension(dimension: u32, scale: f32) -> u32 {
    // `as` saturates, so an absurd scale clamps to `u32::MAX` instead of wrapping.
    ((f64::from(dimension) * f64::from(scale)).round() as u32).max(1)
}

impl fmt::Display for TargetSize {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Absolute { width, height } => write!(formatter, "{width}x{height}"),
            // Exact: 1.0 is how `TargetSize::VIEWPORT` spells the whole viewport.
            Self::Relative { scale: 1.0 } => formatter.write_str("viewport"),
            Self::Relative { scale } => write!(formatter, "viewport*{scale}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Extent = Extent {
        width: 1920,
        height: 1080,
    };

    #[test]
    fn absolute_sizes_ignore_the_viewport() {
        let size = TargetSize::Absolute { width: 256, height: 128 };
        assert_eq!(size.resolve(WINDOW), Extent { width: 256, height: 128 });
    }

    #[test]
    fn relative_sizes_round_to_the_nearest_pixel() {
        assert_eq!(TargetSize::VIEWPORT.resolve(WINDOW), WINDOW);
        assert_eq!(
            TargetSize::Relative { scale: 0.5 }.resolve(WINDOW),
            Extent { width: 960, height: 540 }
        );
        let odd = Extent { width: 1001, height: 3 };
        assert_eq!(
            TargetSize::Relative { scale: 0.5 }.resolve(odd),
            Extent { width: 501, height: 2 }
        );
    }

    #[test]
    fn relative_sizes_never_resolve_to_zero() {
        let minimized = Extent { width: 0, height: 0 };
        assert_eq!(TargetSize::VIEWPORT.resolve(minimized), Extent { width: 1, height: 1 });
        assert_eq!(
            TargetSize::Relative { scale: 0.001 }.resolve(WINDOW),
            Extent { width: 2, height: 1 }
        );
    }

    #[test]
    fn invalid_sizes_are_detected() {
        assert!(TargetSize::VIEWPORT.is_valid());
        assert!(!TargetSize::Absolute { width: 0, height: 1 }.is_valid());
        assert!(!TargetSize::Relative { scale: 0.0 }.is_valid());
        assert!(!TargetSize::Relative { scale: -1.0 }.is_valid());
        assert!(!TargetSize::Relative { scale: f32::NAN }.is_valid());
        assert!(!TargetSize::Relative { scale: f32::INFINITY }.is_valid());
    }

    #[test]
    fn sizes_display_readably() {
        assert_eq!(TargetSize::VIEWPORT.to_string(), "viewport");
        assert_eq!(TargetSize::Relative { scale: 0.5 }.to_string(), "viewport*0.5");
        assert_eq!(TargetSize::Absolute { width: 64, height: 32 }.to_string(), "64x32");
    }

    #[test]
    fn formats_pad_in_columns() {
        assert_eq!(format!("{:<10}|", TextureFormat::R8Unorm), "R8Unorm   |");
        assert!(TextureFormat::Depth24Plus.is_depth());
        assert!(!TextureFormat::R32Uint.is_depth());
        assert!(TextureFormat::R32Uint.is_integer());
    }
}

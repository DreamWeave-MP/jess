//! Buffers and textures: written from bytes, read back as the same bytes, and refused, in Jess
//! terms, where the arguments don't fit.

use super::gpu;

use jess::gpu::{BufferUsage, GpuError, TextureDesc, TextureShape, TextureUsage};
use jess::render::TextureFormat;

#[test]
fn a_buffer_reads_back_what_was_written() {
    let Some(gpu) = gpu("a_buffer_reads_back_what_was_written") else {
        return;
    };
    let info = gpu.info();
    assert!(!info.name.is_empty() && !info.backend.is_empty(), "{info:?}");
    let buffer = gpu.buffer(16, BufferUsage::UNIFORM | BufferUsage::STORAGE).unwrap();
    assert_eq!(buffer.size(), 16);
    assert_eq!(buffer.usage().to_string(), "uniform, storage");
    assert_eq!(gpu.read_buffer(&buffer, 0, 16).unwrap(), [0; 16], "a new buffer is zeroed");
    gpu.write_buffer(&buffer, 4, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    assert_eq!(
        gpu.read_buffer(&buffer, 0, 16).unwrap(),
        [0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0]
    );
    assert_eq!(gpu.read_buffer(&buffer, 8, 4).unwrap(), [5, 6, 7, 8]);

    let refusals = [
        (
            gpu.buffer(6, BufferUsage::VERTEX).unwrap_err(),
            "a buffer of 6 bytes is invalid: expected a multiple of 4 from 4 to",
        ),
        (
            gpu.write_buffer(&buffer, 12, &[0; 8]).unwrap_err(),
            "8 bytes at 12 are invalid in a buffer of 16: expected a multiple of 4 bytes at a multiple of 4, \
             inside it",
        ),
        (
            gpu.read_buffer(&buffer, 2, 4).unwrap_err(),
            "4 bytes at 2 are invalid in a buffer of 16",
        ),
    ];
    for (error, message) in refusals {
        assert!(matches!(error, GpuError::Invalid(_)), "{error:?}");
        assert!(error.to_string().starts_with(message), "{error} doesn't start with {message}");
    }
}

#[test]
fn a_texture_reads_back_what_was_written() {
    let Some(gpu) = gpu("a_texture_reads_back_what_was_written") else {
        return;
    };
    // Three texels wide: a row of 12 bytes, which readback pads to 256 and back.
    let desc = TextureDesc::d2(3, 2, TextureFormat::Rgba8Unorm, TextureUsage::SAMPLED);
    let texture = gpu.texture(&desc).unwrap();
    let texels: Vec<u8> = (0..24).collect();
    gpu.write_texture(&texture, 0, 0, &texels).unwrap();
    assert_eq!(gpu.read_texture(&texture, 0, 0).unwrap(), texels);

    let layered = TextureDesc {
        shape: TextureShape::D2Array { layers: 3 },
        mips: 2,
        ..TextureDesc::d2(4, 4, TextureFormat::R32Uint, TextureUsage::SAMPLED)
    };
    let layers = gpu.texture(&layered).unwrap();
    let ids: Vec<u8> = (0..4u32).flat_map(u32::to_le_bytes).collect();
    gpu.write_texture(&layers, 1, 2, &ids).unwrap();
    assert_eq!(gpu.read_texture(&layers, 1, 2).unwrap(), ids, "level 1 is 2×2");
    assert_eq!(
        gpu.read_texture(&layers, 1, 1).unwrap(),
        [0; 16],
        "the other layers are untouched"
    );

    let refusals = [
        (
            gpu.write_texture(&texture, 0, 0, &[0; 4]).unwrap_err(),
            "4 bytes don't fill level 0 of a 3×2 Rgba8Unorm: expected 24",
        ),
        (
            gpu.read_texture(&layers, 2, 0).unwrap_err(),
            "level 2 of layer 0 is invalid in a 2D array texture of 2 levels and 3 layers",
        ),
        (
            gpu.texture(&TextureDesc {
                shape: TextureShape::Cube,
                ..TextureDesc::d2(4, 2, TextureFormat::Rgba8Unorm, TextureUsage::SAMPLED)
            })
            .unwrap_err(),
            "a 4×2 Rgba8Unorm cube texture is invalid: expected square faces",
        ),
        (
            gpu.texture(&TextureDesc {
                mips: 4,
                ..TextureDesc::d2(4, 4, TextureFormat::Rgba8Unorm, TextureUsage::SAMPLED)
            })
            .unwrap_err(),
            "a 4×4 Rgba8Unorm 2D texture is invalid: expected from 1 mip level to as many as its size halves to",
        ),
        (
            gpu.texture(&TextureDesc::d2(4, 4, TextureFormat::Depth32Float, TextureUsage::TARGET))
                .and_then(|depth| gpu.write_texture(&depth, 0, 0, &[0; 64]))
                .unwrap_err(),
            "a Depth32Float texture can't be written from the CPU: only a pass writes depth",
        ),
    ];
    for (error, message) in refusals {
        assert!(matches!(error, GpuError::Invalid(_)), "{error:?}");
        assert_eq!(error.to_string(), message);
    }
}

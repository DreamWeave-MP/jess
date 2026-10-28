//! Copy passes: what one copies, its region resolved to numbers for a viewport.

use crate::render::error::GraphError;
use crate::render::graph::{RenderGraph, Resource, ResourceDesc, TargetDesc};
use crate::render::pass::{CopyAt, CopyRegion, CopySize, LoadOp, Pass, Subresource};
use crate::render::target::Extent;

/// A texel of one level of one layer of a target.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct TexelAt {
    /// The level of the layer.
    pub subresource: Subresource,
    /// Across.
    pub x: u32,
    /// Down.
    pub y: u32,
}

/// A copy pass's copy, its region resolved to offsets and extents a backend records as they are.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ResolvedCopy {
    /// `bytes` bytes from byte `from` of one buffer to byte `to` of another.
    Buffers {
        /// The source's first byte.
        from: u64,
        /// The destination's first byte.
        to: u64,
        /// How many.
        bytes: u64,
    },
    /// A rectangle of texels between two targets of one format.
    Targets {
        /// The source's first texel.
        from: TexelAt,
        /// The destination's first texel.
        to: TexelAt,
        /// The rectangle.
        extent: Extent,
    },
    /// A rectangle of texels into a buffer, from byte `offset`, each row `row` bytes.
    TargetToBuffer {
        /// The source's first texel.
        from: TexelAt,
        /// The destination's first byte.
        offset: u64,
        /// The bytes a row takes in the buffer, a multiple of
        /// [`COPY_ROW_ALIGNMENT`](crate::render::COPY_ROW_ALIGNMENT).
        row: u32,
        /// The rectangle.
        extent: Extent,
    },
    /// A rectangle of texels from a buffer, from byte `offset`, each row `row` bytes.
    BufferToTarget {
        /// The source's first byte.
        offset: u64,
        /// The bytes a row takes in the buffer.
        row: u32,
        /// The destination's first texel.
        to: TexelAt,
        /// The rectangle.
        extent: Extent,
    },
}

impl RenderGraph {
    /// What copy pass `pass` of this graph copies with `region` at `viewport`. Without a
    /// viewport, a copy of a relative target is checked as far as it can be and resolves to `None`.
    ///
    /// # Errors
    ///
    /// With the problem, when the copy doesn't copy one resource into one it fits: a region of
    /// the wrong kind or out of bounds, targets of two formats, a part of a depth target, a format
    /// with no layout in a buffer, or a buffer into a depth target, which only a pass can write.
    pub fn resolve_copy(
        &self,
        pass: &Pass,
        region: CopyRegion,
        viewport: Option<Extent>,
    ) -> Result<Option<ResolvedCopy>, String> {
        let ([input], [output]) = (&pass.inputs[..], &pass.outputs[..]) else {
            return Err("expected one source and one destination".to_owned());
        };

        if let LoadOp::Clear(_) = output.load {
            return Err("the copy writes its destination, so clearing it first is wasted: load it or don't care".to_owned());
        }

        let source = &self.resources()[input.resource.index()];
        let destination = &self.resources()[output.resource.index()];
        let from = input.one().unwrap_or_default();
        let to = output.subresource;
        match (source.desc, destination.desc) {
            (ResourceDesc::Buffer(_), ResourceDesc::Buffer(_)) => {
                let (Some(held), Some(room)) = (source.bytes_at(viewport), destination.bytes_at(viewport)) else {
                    byte(region.from, "source")?;
                    byte(region.to, "destination")?;
                    if let Some(CopySize::Texels { .. }) = region.size {
                        return Err("a copy between buffers copies bytes, not texels".to_owned());
                    }
                    return Ok(None);
                };
                buffers(source, destination, held, room, region).map(Some)
            }
            (ResourceDesc::Texture(have), ResourceDesc::Texture(room)) => {
                targets((source, have, from), (destination, room, to), region, viewport)
            }
            (ResourceDesc::Texture(target), ResourceDesc::Buffer(_)) => {
                let bytes = destination.bytes_at(viewport);
                let at = TexelAt {
                    subresource: from,
                    ..Default::default()
                };
                let resolved = between(
                    (source, target, at, region.from, "source"),
                    (destination, bytes, region.to, "destination"),
                    region.size,
                    viewport,
                )?;
                Ok(resolved.map(|(from, offset, row, extent)| ResolvedCopy::TargetToBuffer {
                    from,
                    offset,
                    row,
                    extent,
                }))
            }
            (ResourceDesc::Buffer(_), ResourceDesc::Texture(target)) => {
                let bytes = source.bytes_at(viewport);
                if target.format.is_depth() {
                    return Err(format!(
                        "it copies a buffer into \"{}\", a depth target, which only a pass can write",
                        destination.name
                    ));
                }

                let at = TexelAt {
                    subresource: to,
                    ..Default::default()
                };
                let resolved = between(
                    (destination, target, at, region.to, "destination"),
                    (source, bytes, region.from, "source"),
                    region.size,
                    viewport,
                )?;
                Ok(resolved.map(|(to, offset, row, extent)| ResolvedCopy::BufferToTarget { offset, row, to, extent }))
            }
        }
    }

    /// Checks copy pass `pass` as it is registered: as far as it can be without a viewport.
    pub(crate) fn check_copy(&self, pass: &Pass) -> Result<(), GraphError> {
        self.resolve_copy(pass, pass.region, None)
            .map(|_| ())
            .map_err(|problem| GraphError::InvalidCopy {
                pass: pass.name().to_owned(),
                problem,
            })
    }
}

impl Default for TexelAt {
    fn default() -> Self {
        Self {
            subresource: Subresource { level: 0, layer: 0 },
            x: 0,
            y: 0,
        }
    }
}

/// `"name" is size`, and the level where it isn't the first.
fn sized(resource: &Resource, target: TargetDesc, level: u32) -> String {
    if level == 0 {
        return format!("\"{}\" is {}", resource.name, target.size);
    }
    format!("level {level} of \"{}\" is a level of {}", resource.name, target.size)
}

/// A copy between two buffers holding `held` and `room` bytes.
fn buffers(
    source: &Resource,
    destination: &Resource,
    held: u64,
    room: u64,
    region: CopyRegion,
) -> Result<ResolvedCopy, String> {
    let from = byte(region.from, "source")?;
    let to = byte(region.to, "destination")?;
    let bytes = match region.size {
        None => held.saturating_sub(from),
        Some(CopySize::Bytes(bytes)) => bytes,
        Some(CopySize::Texels { .. }) => return Err("a copy between buffers copies bytes, not texels".to_owned()),
    };
    if bytes == 0 {
        return Err(format!(
            "it copies nothing: \"{}\" holds {held} bytes, and it starts at byte {from}",
            source.name
        ));
    }

    if !bytes.is_multiple_of(4) {
        return Err(format!("it copies {bytes} bytes, and a buffer copy copies a multiple of 4"));
    }

    if from.saturating_add(bytes) > held {
        return Err(format!(
            "\"{}\" holds {held} bytes, and {bytes} from byte {from} run past it",
            source.name
        ));
    }

    if to.saturating_add(bytes) > room {
        let at = if to == 0 {
            String::new()
        } else {
            format!(", and the copy lands at byte {to}")
        };
        return Err(format!(
            "\"{}\" holds {held} bytes and \"{}\" only {room}{at}",
            source.name, destination.name
        ));
    }

    Ok(ResolvedCopy::Buffers { from, to, bytes })
}

/// A copy between a target and a buffer, either way: the target's first texel, the buffer's first
/// byte, the bytes a row takes in it, and the rectangle, or `None` without a viewport to size a
/// relative target or a buffer sized by the viewport.
fn between(
    (target, desc, at, start, side): (&Resource, TargetDesc, TexelAt, Option<CopyAt>, &str),
    (buffer, bytes, offset, other): (&Resource, Option<u64>, Option<CopyAt>, &str),
    amount: Option<CopySize>,
    viewport: Option<Extent>,
) -> Result<Option<(TexelAt, u64, u32, Extent)>, String> {
    let format = desc.format;
    let Some(texel_bytes) = format.texel_bytes() else {
        return Err(format!("{format} has no layout in a buffer: the backend chooses it"));
    };

    let (x, y) = texel(start, side)?;
    let offset = byte(offset, other)?;
    if !offset.is_multiple_of(u64::from(texel_bytes)) {
        return Err(format!(
            "it starts at byte {offset} of \"{}\", and a copy with {format} starts at a multiple of {texel_bytes}",
            buffer.name
        ));
    }

    if let Some(CopySize::Bytes(_)) = amount {
        return Err("a copy with a target copies texels, not bytes".to_owned());
    }

    // The target's side of the region, as a depth target's whole copy is checked on.
    let region = CopyRegion {
        from: start,
        to: None,
        size: amount,
    };

    let Some(held) = desc.extent(at.subresource.level, viewport) else {
        whole_depth(desc, region, None)?;
        return Ok(None);
    };

    let extent = texels(amount, held, (x, y), &target.name)?;
    fits(target, at.subresource, held, (x, y), extent)?;
    whole_depth(desc, region, Some((held, extent)))?;
    let row = format
        .copy_row(extent.width)
        .expect("a format with a texel size has a row in a buffer");
    let needed = u64::from(row) * u64::from(extent.height);
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    if offset.saturating_add(needed) > bytes {
        let from = if offset == 0 {
            String::new()
        } else {
            format!(" from byte {offset}")
        };
        return Err(format!(
            "{} at {}×{} takes {needed} bytes in a buffer, rows of {row}, and {} holds {bytes}{from}",
            target.name, extent.width, extent.height, buffer.name
        ));
    }

    Ok(Some((TexelAt { x, y, ..at }, offset, row, extent)))
}

/// Where a region starts in a buffer: a byte, a multiple of 4.
fn byte(at: Option<CopyAt>, side: &str) -> Result<u64, String> {
    match at {
        None => Ok(0),
        Some(CopyAt::Byte(byte)) if byte.is_multiple_of(4) => Ok(byte),
        Some(CopyAt::Byte(byte)) => Err(format!(
            "its {side} starts at byte {byte}, and a buffer copy starts at a multiple of 4"
        )),
        Some(CopyAt::Texel { .. }) => Err(format!("its {side} is a buffer, which starts at a byte, not a texel")),
    }
}

/// Where a region starts in a target: a texel.
fn texel(at: Option<CopyAt>, side: &str) -> Result<(u32, u32), String> {
    match at {
        None => Ok((0, 0)),
        Some(CopyAt::Texel { x, y }) => Ok((x, y)),
        Some(CopyAt::Byte(_)) => Err(format!("its {side} is a target, which starts at a texel, not a byte")),
    }
}

/// The rectangle a region copies out of `held` from `start`: what it names, or the rest.
fn texels(size: Option<CopySize>, held: Extent, start: (u32, u32), name: &str) -> Result<Extent, String> {
    let extent = match size {
        None => Extent {
            width: held.width.saturating_sub(start.0),
            height: held.height.saturating_sub(start.1),
        },
        Some(CopySize::Texels { width, height }) => Extent { width, height },
        Some(CopySize::Bytes(_)) => return Err("a copy with a target copies texels, not bytes".to_owned()),
    };
    if extent.width == 0 || extent.height == 0 {
        return Err(format!(
            "it copies nothing: \"{name}\" is {}×{}, and it starts at texel {}, {}",
            held.width, held.height, start.0, start.1
        ));
    }

    Ok(extent)
}

/// Whether `extent` from `start` fits in `subresource` of `resource`, `held` big.
fn fits(
    resource: &Resource,
    subresource: Subresource,
    held: Extent,
    start: (u32, u32),
    extent: Extent,
) -> Result<(), String> {
    let across = u64::from(start.0) + u64::from(extent.width);
    let down = u64::from(start.1) + u64::from(extent.height);
    if across <= u64::from(held.width) && down <= u64::from(held.height) {
        return Ok(());
    }

    Err(format!(
        "{}×{} texels at {}, {} run past {subresource} of \"{}\", {}×{}",
        extent.width, extent.height, start.0, start.1, resource.name, held.width, held.height
    ))
}

/// Whether a copy with a depth target copies all of a level of a layer, which is all a depth
/// copy can: checked on the region alone without the sizes, `held` and `extent`, to compare.
fn whole_depth(desc: TargetDesc, region: CopyRegion, sizes: Option<(Extent, Extent)>) -> Result<(), String> {
    if !desc.format.is_depth() {
        return Ok(());
    }

    let at_start = |at: Option<CopyAt>| matches!(at, None | Some(CopyAt::Texel { x: 0, y: 0 } | CopyAt::Byte(_)));
    let whole = sizes.is_none_or(|(held, extent)| held == extent);
    if at_start(region.from) && at_start(region.to) && whole {
        return Ok(());
    }

    Err(format!(
        "a copy with a {} target copies a whole level of a layer",
        desc.format
    ))
}

/// A copy between two targets: `source`, `have` at `from`, into `destination`, `room` at `to`.
fn targets(
    (source, have, from): (&Resource, TargetDesc, Subresource),
    (destination, room, to): (&Resource, TargetDesc, Subresource),
    region: CopyRegion,
    viewport: Option<Extent>,
) -> Result<Option<ResolvedCopy>, String> {
    if have.format != room.format {
        return Err(format!(
            "\"{}\" is {} and \"{}\" is {}: a copy between targets keeps the format",
            source.name, have.format, destination.name, room.format
        ));
    }

    let start = texel(region.from, "source")?;
    let end = texel(region.to, "destination")?;
    let (Some(held), Some(space)) = (have.extent(from.level, viewport), room.extent(to.level, viewport)) else {
        whole_depth(have, region, None)?;
        // All of one level into another fits at every viewport only when they are the
        // same size at every viewport.
        if region.is_whole() && have.level_size(from.level) != room.level_size(to.level) {
            return Err(format!(
                "{} and {}: copying all of one into the other needs the same size",
                sized(source, have, from.level),
                sized(destination, room, to.level)
            ));
        }
        return Ok(None);
    };

    let extent = texels(region.size, held, start, &source.name)?;
    fits(source, from, held, start, extent)?;
    fits(destination, to, space, end, extent)?;
    whole_depth(have, region, Some((held, extent)))?;
    whole_depth(room, region, Some((space, extent)))?;
    Ok(Some(ResolvedCopy::Targets {
        from: TexelAt {
            subresource: from,
            x: start.0,
            y: start.1,
        },
        to: TexelAt {
            subresource: to,
            x: end.0,
            y: end.1,
        },
        extent,
    }))
}

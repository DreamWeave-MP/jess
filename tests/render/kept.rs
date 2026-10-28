//! Kept resources, which the plan holds from run to run for the host to read, and buffers sized
//! by the viewport.

use jess::render::{
    BufferSize, ClearValue, Extent, GraphError, LoadOp, Pass, PassKind, PassMetadata, PassOutput, ReadUsage, RenderGraph,
    ResolvedCopy, ResourceDesc, ResourceOrigin, StoreOp, TargetDesc, TargetSize, TexelAt, TextureFormat,
};

fn pass(kind: PassKind, name: &str) -> Pass {
    Pass::new(kind, PassMetadata::new(name, "@test/kept"))
}

#[test]
fn a_kept_resource_is_observed_holds_the_last_run_and_shares_nothing() {
    let mut graph = RenderGraph::new("history");
    let history = graph
        .add(
            "history",
            ResourceDesc::Texture(TargetDesc::new(TextureFormat::Rgba16Float, TargetSize::VIEWPORT)),
            ResourceOrigin::Kept,
        )
        .unwrap();
    let scratch = graph
        .add_render_target("scratch", TextureFormat::Rgba16Float, TargetSize::VIEWPORT)
        .unwrap();
    let draw = |resource, load| PassOutput::color(resource, load, StoreOp::Store);
    graph
        .add_pass(pass(PassKind::Fullscreen, "scene").write(draw(scratch, LoadOp::DontCare)))
        .unwrap();
    // The first write loads what the last run left: a kept resource has contents.
    graph
        .add_pass(
            pass(PassKind::Fullscreen, "accumulate")
                .read(scratch, ReadUsage::Texture)
                .write(draw(history, LoadOp::Load)),
        )
        .unwrap();
    let compiled = graph.compile().unwrap();
    let order: Vec<&str> = compiled.order().iter().map(|&id| graph.pass(id).unwrap().name()).collect();
    assert_eq!(
        order,
        ["scene", "accumulate"],
        "nothing is imported, and still nothing is culled"
    );
    assert_eq!(compiled.alias_groups().len(), 1, "the scratch target alone");
    let dump = compiled.to_string();
    assert!(
        dump.contains("history  Rgba16Float  viewport  kept       accumulate"),
        "{dump}"
    );
}

#[test]
fn a_buffer_sized_by_the_viewport_holds_a_copy_of_a_target() {
    let mut graph = RenderGraph::new("readback");
    let color = graph
        .add_render_target("color", TextureFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let image = BufferSize::Image {
        format: TextureFormat::Rgba8Unorm,
        scale: 1.0,
    };
    let pixels = graph
        .add("pixels", ResourceDesc::Buffer(image), ResourceOrigin::Kept)
        .unwrap();
    graph
        .add_pass(pass(PassKind::Fullscreen, "draw").write(PassOutput::color(
            color,
            LoadOp::Clear(ClearValue::Color([0.0; 4])),
            StoreOp::Store,
        )))
        .unwrap();
    let copy = pass(PassKind::Copy, "readBack")
        .read(color, ReadUsage::Copy)
        .write(PassOutput::copy(pixels, LoadOp::DontCare, StoreOp::Store));
    graph.add_pass(copy.clone()).unwrap();

    // 100 texels of 4 bytes pad to 512 a row.
    let viewport = Extent { width: 100, height: 3 };
    assert_eq!(image.resolve(viewport), 512 * 3);
    assert_eq!(image.to_string(), "Rgba8Unorm rows at viewport");
    assert_eq!(graph.resolve_copy(&copy, copy.region, None), Ok(None));
    assert_eq!(
        graph.resolve_copy(&copy, copy.region, Some(viewport)),
        Ok(Some(ResolvedCopy::TargetToBuffer {
            from: TexelAt::default(),
            offset: 0,
            row: 512,
            extent: viewport,
        }))
    );

    let invalid = |size| {
        graph
            .clone()
            .add("bad", ResourceDesc::Buffer(size), ResourceOrigin::Transient)
            .unwrap_err()
    };
    assert_eq!(
        invalid(BufferSize::Image {
            format: TextureFormat::Depth24Plus,
            scale: 1.0
        }),
        GraphError::InvalidBufferShape {
            resource: "bad".to_owned(),
            problem: "a format with a layout in a buffer, not Depth24Plus".to_owned(),
        }
    );
    assert_eq!(
        invalid(BufferSize::Image {
            format: TextureFormat::R8Unorm,
            scale: 0.0
        })
        .to_string(),
        "buffer \"bad\" needs a finite, positive scale, not 0"
    );
}

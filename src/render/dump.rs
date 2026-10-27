//! The debug dump: [`CompiledGraph`]'s `Display`.

use std::fmt;

use crate::render::compile::CompiledGraph;
use crate::render::graph::{ResourceDesc, ResourceId};
use crate::render::pass::Subresource;

impl fmt::Display for CompiledGraph<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let graph = self.graph;
        writeln!(formatter, "{}", graph.name())?;
        for pass in &self.order {
            let pass = &graph.passes()[pass.0];
            writeln!(formatter)?;
            writeln!(formatter, "{}", pass.name())?;
            if let Some(source) = &pass.metadata.source {
                writeln!(formatter, "    from:   {source}")?;
            }
            if !pass.inputs.is_empty() {
                formatter.write_str("    reads:  ")?;
                let reads = pass.inputs.iter().map(|input| (input.resource, input.subresource, None));
                self.write_uses(formatter, reads)?;
            }

            formatter.write_str("    writes: ")?;
            let writes = pass
                .outputs
                .iter()
                .map(|output| (output.resource, Some(output.subresource), output.resolve));
            self.write_uses(formatter, writes)?;
        }

        if !self.culled.is_empty() {
            writeln!(formatter)?;
            writeln!(formatter, "culled")?;
            for pass in &self.culled {
                writeln!(formatter, "    {}", graph.passes()[pass.0].name())?;
            }
        }

        self.write_resources(formatter)?;
        if self.alias_groups.is_empty() {
            return Ok(());
        }

        writeln!(formatter)?;
        writeln!(formatter, "aliasing")?;
        for group in &self.alias_groups {
            formatter.write_str("    ")?;
            self.write_names(formatter, group.resources.iter().copied())?;
        }

        Ok(())
    }
}

impl CompiledGraph<'_> {
    /// Each resource a pass uses, the level of the layer where its resource has more than one,
    /// and where it resolves to.
    fn write_uses(
        &self,
        formatter: &mut fmt::Formatter<'_>,
        uses: impl Iterator<Item = (ResourceId, Option<Subresource>, Option<(ResourceId, Subresource)>)>,
    ) -> fmt::Result {
        for (index, (resource, subresource, resolve)) in uses.enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }

            self.write_use(formatter, resource, subresource)?;
            if let Some((target, subresource)) = resolve {
                formatter.write_str(" -> ")?;
                self.write_use(formatter, target, Some(subresource))?;
            }
        }

        writeln!(formatter)
    }

    fn write_use(
        &self,
        formatter: &mut fmt::Formatter<'_>,
        resource: ResourceId,
        subresource: Option<Subresource>,
    ) -> fmt::Result {
        let resource = &self.graph.resources()[resource.0];
        formatter.write_str(&resource.name)?;
        let (Some(target), Some(subresource)) = (resource.target(), subresource) else {
            return Ok(());
        };

        match (target.mips > 1, target.layers > 1) {
            (true, true) => write!(formatter, "[level {}, layer {}]", subresource.level, subresource.layer),
            (true, false) => write!(formatter, "[level {}]", subresource.level),
            (false, true) => write!(formatter, "[layer {}]", subresource.layer),
            (false, false) => Ok(()),
        }
    }

    fn write_names(&self, formatter: &mut fmt::Formatter<'_>, resources: impl Iterator<Item = ResourceId>) -> fmt::Result {
        for (index, resource) in resources.enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }

            formatter.write_str(&self.graph.resources()[resource.0].name)?;
        }

        writeln!(formatter)
    }

    /// One row per resource, in registration order: name, format, size, origin, then the passes
    /// that bound its lifetime.
    fn write_resources(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let resources = self.graph.resources();
        if resources.is_empty() {
            return Ok(());
        }

        let formats: Vec<&str> = resources
            .iter()
            .map(|resource| resource.format().map_or("buffer", |format| format.name()))
            .collect();
        let sizes: Vec<String> = resources
            .iter()
            .map(|resource| match resource.desc {
                ResourceDesc::Texture(target) => {
                    let extras = [
                        (target.mips, "levels"),
                        (target.layers, "layers"),
                        (target.samples, "samples"),
                    ]
                    .into_iter()
                    .filter(|&(count, _)| count > 1)
                    .map(|(count, what)| format!(", {count} {what}"))
                    .collect::<Vec<_>>()
                    .concat();
                    format!("{}{extras}", target.size)
                }
                ResourceDesc::Buffer(size) => size.to_string(),
            })
            .collect();
        let name_width = resources.iter().map(|resource| resource.name.len()).max().unwrap_or(0);
        let format_width = formats.iter().map(|format| format.len()).max().unwrap_or(0);
        let size_width = sizes.iter().map(String::len).max().unwrap_or(0);
        let origin_width = resources
            .iter()
            .map(|resource| resource.origin.name().len())
            .max()
            .unwrap_or(0);

        writeln!(formatter)?;
        writeln!(formatter, "resources")?;
        for (index, resource) in resources.iter().enumerate() {
            write!(
                formatter,
                "    {:<name_width$}  {:<format_width$}  {:<size_width$}  {:<origin_width$}  ",
                resource.name,
                formats[index],
                sizes[index],
                resource.origin.name(),
            )?;

            let Some(lifetime) = self.lifetimes[index] else {
                let referenced = self
                    .graph
                    .passes()
                    .iter()
                    .any(|pass| pass.resources().any(|resource| resource.0 == index));
                writeln!(formatter, "{}", if referenced { "culled" } else { "unused" })?;
                continue;
            };

            let first = self.graph.passes()[self.order[lifetime.first].0].name();
            if lifetime.first == lifetime.last {
                writeln!(formatter, "{first}")?;
                continue;
            }

            let last = self.graph.passes()[self.order[lifetime.last].0].name();
            writeln!(formatter, "{first} .. {last}")?;
        }

        Ok(())
    }
}

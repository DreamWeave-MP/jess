//! The debug dump: [`CompiledGraph`]'s `Display`.

use std::fmt;

use crate::render::compile::CompiledGraph;
use crate::render::graph::ResourceId;

impl fmt::Display for CompiledGraph<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let graph = self.graph;
        writeln!(formatter, "{}", graph.name())?;
        for pass in &self.order {
            let pass = &graph.passes()[pass.0];
            writeln!(formatter)?;
            writeln!(formatter, "{}", pass.name())?;
            if !pass.inputs.is_empty() {
                formatter.write_str("    reads:  ")?;
                self.write_names(formatter, pass.inputs.iter().map(|input| input.resource))?;
            }

            formatter.write_str("    writes: ")?;
            self.write_names(formatter, pass.outputs.iter().map(|output| output.resource))?;
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
    fn write_names(&self, formatter: &mut fmt::Formatter<'_>, resources: impl Iterator<Item = ResourceId>) -> fmt::Result {
        for (index, resource) in resources.enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }

            formatter.write_str(&self.graph.resources()[resource.0].name)?;
        }

        writeln!(formatter)
    }

    /// One row per target, in registration order: name, format, size, origin, then the passes
    /// that bound its lifetime.
    fn write_resources(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let resources = self.graph.resources();
        if resources.is_empty() {
            return Ok(());
        }

        let sizes: Vec<String> = resources.iter().map(|resource| resource.size.to_string()).collect();
        let name_width = resources.iter().map(|resource| resource.name.len()).max().unwrap_or(0);
        let format_width = resources
            .iter()
            .map(|resource| resource.format.name().len())
            .max()
            .unwrap_or(0);
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
                resource.format,
                sizes[index],
                resource.origin.name(),
            )?;

            let Some(lifetime) = self.lifetimes[index] else {
                let referenced = self.graph.passes().iter().any(|pass| {
                    pass.inputs.iter().any(|input| input.resource.0 == index)
                        || pass.outputs.iter().any(|output| output.resource.0 == index)
                });
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

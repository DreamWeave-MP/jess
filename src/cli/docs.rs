//! `jess docs`: the guide, as the binary carries it, for a machine with no route to the site.
//!
//! The pages are the site's own sources, embedded when jess is built. The API reference isn't
//! among them: it is generated from jess's source documentation, which `jess luau-types` writes
//! beside the types, each module's with its stub.

use std::io::Write;

use clap::Args;

include!(concat!(env!("OUT_DIR"), "/guide.rs"));

#[derive(Debug, Args)]
pub struct DocsArgs {
    /// The page to print, as jess docs lists it (scripting/applications). Without one, the list.
    page: Option<String>,
}

/// What marks a page whose body the site generates from jess's source documentation.
const GENERATED: &str = "<!-- Generated from the API documentation in Jess source. -->";

/// Where a link into the site's sources starts.
const LINK: &str = "](@/";

/// A page's front matter value for `key`, as it reads: without its quotes, and with the quote
/// YAML doubles inside a quoted value single again.
fn field(source: &str, key: &str) -> Option<String> {
    let matter = source.strip_prefix("---\n")?.split("\n---\n").next()?;
    let value = matter
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))?
        .trim();
    let quoted = |quote: char| value.strip_prefix(quote).and_then(|rest| rest.strip_suffix(quote));
    Some(quoted('\'').map_or_else(|| quoted('"').unwrap_or(value).to_owned(), |inner| inner.replace("''", "'")))
}

/// A page's text after its front matter.
fn body(source: &str) -> &str {
    source
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map_or(source, |(_, body)| body)
        .trim_start()
}

/// What stands for a link to `target`, a path under the site's content: the command that prints
/// a page of the guide, or where the API reference and the rest of the site are read.
fn pointer(target: &str) -> String {
    let path = target.split('#').next().unwrap_or_default();
    let Some(page) = path.strip_prefix("docs/") else {
        return " (on the site)".to_owned();
    };
    let page = page.trim_end_matches(".md").trim_end_matches("/_index");
    if page == "api" || page.starts_with("api/") {
        return " (API reference: in the types jess luau-types writes)".to_owned();
    }
    format!(" (jess docs {page})")
}

/// One line of prose with what only the site's generator reads turned into what a terminal can:
/// a link into the site becomes where to read its target, and a shortcode the code it names.
fn plain_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    loop {
        let (linked, code) = (rest.find(LINK), rest.find("{{ "));
        match (linked, code) {
            (Some(at), code) if code.is_none_or(|code| at < code) => {
                out.push_str(&rest[..=at]);
                let target = &rest[at + LINK.len()..];
                let end = target.find(')').unwrap_or(target.len());
                out.push_str(&pointer(&target[..end]));
                rest = target.get(end + 1..).unwrap_or_default();
            }
            (_, Some(at)) => {
                out.push_str(&rest[..at]);
                let call = &rest[at..];
                let end = call.find("}}").map_or(call.len(), |end| end + 2);
                out.push('`');
                out.push_str(call[..end].split('"').nth(1).unwrap_or_default());
                out.push('`');
                rest = &call[end..];
            }
            _ => break,
        }
    }
    out.push_str(rest);
    out
}

/// A page's body as a terminal reads it. Code is left exactly as written: only prose outside a
/// fence is rewritten, and a block shortcode's own lines become its title, or nothing.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if fenced || line.trim_start().starts_with("```") {
            out.push_str(line);
        } else if let Some(shortcode) = line.trim().strip_prefix("{% ") {
            if let Some(title) = shortcode.split('"').nth(1) {
                out.push_str("**");
                out.push_str(title);
                out.push_str("**");
            }
        } else {
            out.push_str(&plain_line(line));
        }
        out.push('\n');
    }
    out
}

/// The pages that have a guide to print, by the name `jess docs` takes.
fn pages() -> impl Iterator<Item = (&'static str, &'static str)> {
    PAGES
        .iter()
        .map(|(path, source)| match *path {
            "_index" => ("index", *source),
            path => (path.trim_end_matches("/_index"), *source),
        })
        .filter(|(_, source)| !source.contains(GENERATED))
}

pub fn run(args: &DocsArgs) -> Result<(), String> {
    let mut out = crate::output::stdout();
    let written = |result: std::io::Result<()>| result.map_err(|error| format!("writing the guide: {error}"));
    let wanted = args
        .page
        .as_deref()
        .map(|page| page.trim_matches('/').trim_end_matches(".md"))
        .filter(|page| !page.is_empty());
    let Some(wanted) = wanted else {
        written(writeln!(
            out,
            "The jess guide, as this binary carries it. Print a page with: jess docs <page>\n"
        ))?;
        for (name, source) in pages() {
            written(writeln!(
                out,
                "{name}\n    {}",
                field(source, "description").unwrap_or_default()
            ))?;
        }
        return written(writeln!(
            out,
            "\nThe API reference is in the types jess writes: jess luau-types <folder> puts jess's own in \
             <folder>/jess.d.jsl, and each module's with its stub under <folder>/jess and <folder>/dream."
        ));
    };
    let Some((_, source)) = pages().find(|(name, _)| *name == wanted) else {
        let word = wanted.rsplit('/').next().unwrap_or(wanted);
        let near: Vec<&str> = pages()
            .map(|(name, _)| name)
            .filter(|name| name.rsplit('/').next().is_some_and(|last| last.contains(word)))
            .collect();
        let hint = if near.is_empty() || near.len() > 5 {
            "jess docs lists the pages".to_owned()
        } else {
            format!("did you mean {}?", near.join(", "))
        };
        return Err(format!("no page called {wanted}: {hint}"));
    };
    let title = field(source, "title").unwrap_or_else(|| wanted.to_owned());
    written(write!(out, "# {title}\n\n{}", plain(body(source))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_reads_in_a_terminal() {
        let source = "---\ntitle: 'A page''s title'\ndescription: Plain words.\nweight: 1\n---\n\nSee [the guide](@/docs/scripting/applications.md#windows), \
                      [packages](@/docs/scripting/_index.md), [`jess.gpu`](@/docs/api/gpu.md#windows), and [the rules](@/governance.md).\n\n\
                      {{ api_signature(value=\"jess run <SCRIPT>\") }}\n\n{% usage_note(title=\"Take care\") %}\nWith this.\n{% end %}\n\n\
                      ```luau\nlocal nested = {{ 1 }}\n-- [a link](@/docs/cli/run.md) in code\n```\n";
        assert_eq!(field(source, "title").as_deref(), Some("A page's title"));
        assert_eq!(field(source, "description").as_deref(), Some("Plain words."));
        assert_eq!(
            plain(body(source)),
            "See [the guide] (jess docs scripting/applications), [packages] (jess docs scripting), [`jess.gpu`] (API \
             reference: in the types jess luau-types writes), and [the rules] (on the site).\n\n`jess run <SCRIPT>`\n\n\
             **Take care**\nWith this.\n\n\n```luau\nlocal nested = {{ 1 }}\n-- [a link](@/docs/cli/run.md) in code\n```\n"
        );
    }

    /// Every page a page of the guide sends its reader to with `jess docs` is one it carries, and
    /// nothing only the site's generator reads is left in prose.
    #[test]
    fn every_page_the_guide_points_at_is_carried() {
        let names: Vec<&str> = pages().map(|(name, _)| name).collect();
        for expected in [
            "index",
            "scripting/applications",
            "cookbook/window",
            "reference/manifest",
            "api",
        ] {
            assert!(names.contains(&expected), "{expected} is missing from {names:?}");
        }
        assert!(!names.contains(&"api/gpu"), "the generated reference lives in the types");
        assert!(names.iter().all(|name| !name.contains('\\')), "{names:?}");
        for (name, source) in pages() {
            assert!(
                field(source, "title").is_some_and(|title| !title.is_empty()),
                "{name} has no title"
            );
            let text = plain(body(source));
            for pointed in text.split("(jess docs ").skip(1) {
                let page = pointed.split(')').next().unwrap_or_default();
                assert!(names.contains(&page), "{name} points at {page}, which isn't carried");
            }
            let prose: String = text.split("```").step_by(2).collect();
            assert!(
                !prose.contains(LINK) && !prose.contains("{% "),
                "{name} keeps site syntax in prose"
            );
        }
    }
}

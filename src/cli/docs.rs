//! `jess docs`: the guide, as the binary carries it, for a machine with no route to the site.
//!
//! The pages are the site's own sources, embedded when jess is built. The API reference isn't
//! among them: it is generated from jess's source documentation, which `jess luau-types` writes
//! into `jess.d.jsl` beside the types.

use clap::Args;

include!(concat!(env!("OUT_DIR"), "/guide.rs"));

#[derive(Debug, Args)]
pub struct DocsArgs {
    /// The page to print, as jess docs lists it (scripting/applications). Without one, the list.
    page: Option<String>,
}

/// What marks a page whose body the site generates from jess's source documentation.
const GENERATED: &str = "<!-- Generated from the API documentation in Jess source. -->";

/// A page's front matter value for `key`, without its quotes.
fn field<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    let matter = source.strip_prefix("---\n")?.split("\n---\n").next()?;
    let value = matter
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))?
        .trim();
    Some(value.strip_prefix('\'')?.strip_suffix('\'').unwrap_or(value))
}

/// A front matter value as it reads: YAML doubles a quote inside a quoted one.
fn unquoted(value: &str) -> String {
    value.replace("''", "'")
}

/// A page's text after its front matter.
fn body(source: &str) -> &str {
    source
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map_or(source, |(_, body)| body)
        .trim_start()
}

/// `text` with what only the site's generator reads turned into what a terminal can: a link to
/// another page becomes the command that prints it, and a shortcode becomes the code it names.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let link = rest.find("](@/docs/");
        let code = rest.find("{{ ");
        match (link, code) {
            (Some(at), code) if code.is_none_or(|code| at < code) => {
                out.push_str(&rest[..at]);
                let target = &rest[at + "](@/docs/".len()..];
                let end = target.find(')').unwrap_or(target.len());
                let page = target[..end].split('#').next().unwrap_or_default();
                let page = page.trim_end_matches(".md").trim_end_matches("/_index");
                out.push_str("] (jess docs ");
                out.push_str(page);
                out.push(')');
                rest = target.get(end + 1..).unwrap_or_default();
            }
            (_, Some(at)) => {
                out.push_str(&rest[..at]);
                let call = &rest[at..];
                let end = call.find("}}").map_or(call.len(), |end| end + 2);
                let named = call[..end].split('"').nth(1).unwrap_or_default();
                out.push('`');
                out.push_str(named);
                out.push('`');
                rest = &call[end..];
            }
            _ => break,
        }
    }
    out.push_str(rest);
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
        .filter(|(name, source)| !source.contains(GENERATED) && *name != "api" && !name.starts_with("api/"))
}

pub fn run(args: &DocsArgs) -> Result<(), String> {
    let Some(wanted) = &args.page else {
        println!("The jess guide, as this binary carries it. Print a page with: jess docs <page>\n");
        for (name, source) in pages() {
            println!("{name}\n    {}", unquoted(field(source, "description").unwrap_or_default()));
        }
        println!(
            "\nThe API reference is in the types jess writes: jess luau-types <folder>, then read \
             <folder>/jess.d.jsl."
        );
        return Ok(());
    };
    let wanted = wanted.trim_end_matches(".md").trim_matches('/');
    let Some((_, source)) = pages().find(|(name, _)| *name == wanted) else {
        let word = wanted.rsplit('/').next().unwrap_or(wanted);
        let near: Vec<&str> = pages().map(|(name, _)| name).filter(|name| name.contains(word)).collect();
        let hint = if near.is_empty() {
            "jess docs lists the pages".to_owned()
        } else {
            format!("did you mean {}?", near.join(", "))
        };
        return Err(format!("no page called {wanted}: {hint}"));
    };
    println!("# {}\n", unquoted(field(source, "title").unwrap_or(wanted)));
    print!("{}", plain(body(source)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_reads_in_a_terminal() {
        let source = "---\ntitle: 'A page'\ndescription: 'What it is.'\nweight: 1\n---\n\nSee [the guide](@/docs/scripting/applications.md#windows) and \
                      [packages](@/docs/scripting/_index.md).\n\n{{ api_signature(value=\"jess run <SCRIPT>\") }}\n";
        assert_eq!(field(source, "title"), Some("A page"));
        assert_eq!(field(source, "description"), Some("What it is."));
        assert_eq!(
            plain(body(source)),
            "See [the guide] (jess docs scripting/applications) and [packages] (jess docs scripting).\n\n`jess run <SCRIPT>`\n"
        );
    }

    #[test]
    fn the_binary_carries_the_guide_and_not_the_generated_reference() {
        let names: Vec<&str> = pages().map(|(name, _)| name).collect();
        for expected in [
            "scripting/applications",
            "cookbook/window",
            "cookbook/worker",
            "reference/manifest",
        ] {
            assert!(names.contains(&expected), "{expected} is missing from {names:?}");
        }
        assert!(
            !names.contains(&"api/gpu"),
            "the API reference is generated, and lives in the types"
        );
        assert!(names.iter().all(|name| !name.contains('\\')), "{names:?}");
    }
}

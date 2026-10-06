//! `jess.regex`: Rust regular expressions for scripts, since Luau patterns aren't regexes.
//!
//! Patterns use the `regex` crate's syntax. Text is matched as bytes, so any Luau string works,
//! and positions are 1-based byte positions with inclusive ends, like `string.find`'s.

use std::rc::Rc;

use l3i::Runtime;
use l3i::bind::{ArgView, Call, StackResults};
use l3i::extension::{ExtensionDescriptor, TagPolicy};
use l3i::stack::{Frame, Scope, ValueView};
use l3i::userdata::{Owned, Userdata};
use l3i::value::{Function, Table, Value};
use regex::bytes::{Captures, Regex, RegexBuilder};

use crate::lua::plan::Functions;
use crate::lua::{Jess, Opts, Text, error, kind, lua};

/// A compiled regex.
pub struct LuaRegex(Regex);

// SAFETY: plain Rust data; the destructor never touches the Lua API.
unsafe impl Userdata for LuaRegex {
    const NAME: &'static str = "jess.Regex";
}

pub fn module(runtime: &Runtime, _jess: &Rc<Jess>) -> l3i::Result<Table> {
    let module = Functions::module(runtime, "regex")?;
    module.function(
        "new",
        |call: &Call<'_>, pattern: Text, options: Opts| -> l3i::Result<Owned<LuaRegex>> {
            options.check(call, "jess.regex.new", &["caseInsensitive", "dotAll", "multiLine"])?;
            let flag = |name: &str| -> l3i::Result<bool> { Ok(options.get::<bool>(call, name)?.unwrap_or(false)) };
            RegexBuilder::new(&pattern.0)
                .case_insensitive(flag("caseInsensitive")?)
                .multi_line(flag("multiLine")?)
                .dot_matches_new_line(flag("dotAll")?)
                .build()
                .map(|regex| Owned(LuaRegex(regex)))
                .map_err(|source| error(source.to_string()))
        },
    )?;
    module.function("escape", |text: Text| regex::escape(&text.0))?;
    module.finish()
}

/// A 1-based start position as a byte offset; positions past the end start at the end.
fn start_offset(text: &[u8], init: Option<i64>) -> usize {
    match init {
        None => 0,
        Some(init) if init > 0 => usize::try_from(init - 1).unwrap_or(usize::MAX).min(text.len()),
        // Negative positions count from the end, like `string.find`'s.
        Some(init) => text
            .len()
            .saturating_sub(usize::try_from(init.unsigned_abs()).unwrap_or(usize::MAX)),
    }
}

/// Pushes a match's groups: `[0]` the whole match, `[n]` group n, and named groups by name; groups
/// that didn't take part are missing.
fn push_captures(scope: &impl Scope, regex: &Regex, captures: &Captures<'_>) -> l3i::Result<()> {
    lua::push_table(scope, captures.len(), 0, |frame, table| {
        for (index, group) in captures.iter().enumerate() {
            if let Some(group) = group {
                frame.push(group.as_bytes())?;
                table.raw_set_index(frame, index as i64)?;
            }
        }
        for name in regex.capture_names().flatten() {
            if let Some(group) = captures.name(name) {
                frame.push(group.as_bytes())?;
                table.raw_set(frame, name)?;
            }
        }
        Ok(())
    })
}

/// What a match is replaced with: a string (`$1`, `${name}` expand), or a function given the
/// match's captures table, whose string result replaces it (`nil` or `false` keeps the match, which
/// then doesn't count as replaced, or toward a limit).
enum Replacement {
    Text(Vec<u8>),
    Function(Function),
}

impl Replacement {
    fn from_value(value: ValueView<'_>) -> l3i::Result<Self> {
        if value.is_string() {
            return Ok(Self::Text(value.read::<&[u8]>()?.to_vec()));
        }
        if value.is_function() {
            return Ok(Self::Function(Function::from_value(Value::store(value)?)?));
        }
        Err(error(format!("a replacement is a string or a function, not {}", kind(value))))
    }

    fn replace(&self, frame: &Frame<'_>, regex: &Regex, text: &[u8], limit: usize) -> l3i::Result<(Vec<u8>, usize)> {
        let mut out = Vec::with_capacity(text.len());
        let mut last = 0;
        let mut count = 0;
        for captures in regex.captures_iter(text) {
            if limit != 0 && count == limit {
                break;
            }
            let whole = captures.get(0).expect("group 0 is the whole match");
            out.extend_from_slice(&text[last..whole.start()]);
            match self {
                Self::Text(replacement) => {
                    captures.expand(replacement, &mut out);
                    count += 1;
                }
                Self::Function(function) => {
                    let replaced = frame.with_frame(|frame| {
                        push_captures(frame, regex, &captures)?;
                        let groups = frame.top_value();
                        function
                            .invoke_with(frame, (groups,), |_, returned| {
                                if returned.is_nil() || (returned.is_boolean() && !returned.read::<bool>()?) {
                                    return Ok(None);
                                }
                                if returned.is_string() {
                                    return Ok(Some(returned.read::<&[u8]>()?.to_vec()));
                                }
                                Err(error(format!(
                                    "a replacement function returns a string, nil, or false, not {}",
                                    kind(returned)
                                )))
                            })
                            .map_err(crate::lua::callback_error)
                    })?;
                    match replaced {
                        Some(replacement) => {
                            out.extend_from_slice(&replacement);
                            count += 1;
                        }
                        None => out.extend_from_slice(whole.as_bytes()),
                    }
                }
            }
            last = whole.end();
        }
        out.extend_from_slice(&text[last..]);
        Ok((out, count))
    }
}

/// Declares the regex type.
#[allow(clippy::too_many_lines)]
pub fn describe(d: &mut ExtensionDescriptor) {
    let mut regex = d.userdata::<LuaRegex>(LuaRegex::NAME);
    regex.tag(TagPolicy::Never).doc("A compiled regular expression.");
    regex
        .getter("pattern", |this: &LuaRegex| this.0.as_str().to_owned())
        .untyped();
    regex
        .method("isMatch", |this: &LuaRegex, text: &[u8]| this.0.is_match(text))
        .untyped();
    regex
        .method(
            "find",
            |this: &LuaRegex, call: &Call<'_>, text: &[u8], init: Option<i64>| -> l3i::Result<StackResults> {
                let start = start_offset(text, init);
                if let Some(found) = this.0.find_at(text, start) {
                    call.push(&(found.start() + 1))?;
                    call.push(&found.end())?;
                    call.push(found.as_bytes())?;
                } else {
                    call.push(&())?;
                    call.push(&())?;
                    call.push(&())?;
                }
                Ok(StackResults)
            },
        )
        .untyped();
    regex
        .method(
            "match",
            |this: &LuaRegex, text: &[u8], init: Option<i64>| -> Option<Vec<u8>> {
                this.0
                    .find_at(text, start_offset(text, init))
                    .map(|found| found.as_bytes().to_vec())
            },
        )
        .untyped();
    regex
        .method(
            "captures",
            |this: &LuaRegex, call: &Call<'_>, text: &[u8], init: Option<i64>| -> l3i::Result<StackResults> {
                match this.0.captures_at(text, start_offset(text, init)) {
                    Some(captures) => push_captures(call, &this.0, &captures)?,
                    None => {
                        call.push(&())?;
                    }
                }
                Ok(StackResults)
            },
        )
        .untyped();
    regex
        .method(
            "matches",
            |this: &LuaRegex, call: &Call<'_>, text: &[u8]| -> l3i::Result<Function> {
                let text = text.to_vec();
                let regex = this.0.clone();
                let position = std::cell::Cell::new(0usize);
                let done = std::cell::Cell::new(false);
                lua::function(
                    call,
                    "jess.Regex.next",
                    move |call: &Call<'_>, _: ArgView<'_>| -> l3i::Result<StackResults> {
                        if done.get() {
                            return Ok(StackResults);
                        }
                        let Some(found) = regex.find_at(&text, position.get()) else {
                            done.set(true);
                            return Ok(StackResults);
                        };
                        // An empty match moves on a byte, so iteration always ends.
                        position.set(if found.is_empty() { found.end() + 1 } else { found.end() });
                        done.set(position.get() > text.len());
                        call.push(found.as_bytes())?;
                        call.push(&(found.start() + 1))?;
                        call.push(&found.end())?;
                        Ok(StackResults)
                    },
                )
            },
        )
        .untyped();
    regex
        .method(
            "replace",
            |this: &LuaRegex, call: &Call<'_>, text: &[u8], replacement: ValueView<'_>| -> l3i::Result<(Vec<u8>, usize)> {
                let replacement = Replacement::from_value(replacement)?;
                call.with_frame(|frame| replacement.replace(frame, &this.0, text, 1))
            },
        )
        .untyped();
    regex
        .method(
            "replaceAll",
            |this: &LuaRegex,
             call: &Call<'_>,
             text: &[u8],
             replacement: ValueView<'_>,
             limit: Option<usize>|
             -> l3i::Result<(Vec<u8>, usize)> {
                let replacement = Replacement::from_value(replacement)?;
                call.with_frame(|frame| replacement.replace(frame, &this.0, text, limit.unwrap_or(0)))
            },
        )
        .untyped();
    regex
        .method(
            "split",
            |this: &LuaRegex, call: &Call<'_>, text: &[u8], limit: Option<usize>| -> l3i::Result<StackResults> {
                let pieces: Vec<&[u8]> = match limit {
                    Some(limit) if limit > 0 => this.0.splitn(text, limit).collect(),
                    _ => this.0.split(text).collect(),
                };
                lua::push_list(call, pieces.into_iter(), |frame, piece| {
                    frame.push(piece)?;
                    Ok(())
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
    crate::lua::plan::metamethod(&mut regex, "__tostring", |this: &LuaRegex| this.0.as_str().to_owned());
}

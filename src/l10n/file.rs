//! The bundle file format: one TOML table of message ids to patterns.
//!
//! ```toml
//! # Comments survive translators.
//! hello = "Hello, {name}!"
//! file_count = '''
//! {count, plural,
//!   =0 {No files}
//!   one {# file}
//!   other {# files}}'''
//! ```
//!
//! TOML is an implementation detail of this module: nothing public names it, and the registry
//! sees only ids, patterns, and where each came from.

use toml::de::{DeTable, DeValue};

/// One message as a file spells it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RawMessage {
    pub(crate) id: String,
    pub(crate) pattern: String,
    /// Where the id is, 1-based.
    pub(crate) line: u32,
    pub(crate) column: u32,
    /// The byte offset of the pattern's text in the file, when the file spells the pattern
    /// verbatim (no escapes), so an error inside the pattern can be placed exactly.
    pub(crate) pattern_offset: Option<usize>,
}

/// Something wrong with a bundle file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileError {
    pub(crate) line: u32,
    pub(crate) column: u32,
    pub(crate) reason: String,
    /// The message the problem belongs to, when it has one.
    pub(crate) message: Option<String>,
    /// The file defines one id twice.
    pub(crate) duplicate: bool,
}

/// Where each line of a file starts, so a byte offset becomes a line and column in one binary
/// search instead of a scan from the top.
pub struct LineIndex<'s> {
    source: &'s str,
    starts: Vec<usize>,
}

impl<'s> LineIndex<'s> {
    pub(crate) fn new(source: &'s str) -> Self {
        let starts = std::iter::once(0)
            .chain(
                source
                    .bytes()
                    .enumerate()
                    .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
            )
            .collect();
        Self { source, starts }
    }

    /// The 1-based line and column (in characters) of a byte offset.
    pub(crate) fn position(&self, offset: usize) -> (u32, u32) {
        let offset = offset.min(self.source.len());
        let line = self.starts.partition_point(|start| *start <= offset);
        let line_start = self.starts[line - 1];
        let column = self.source[line_start..offset].chars().count() + 1;
        (
            u32::try_from(line).unwrap_or(u32::MAX),
            u32::try_from(column).unwrap_or(u32::MAX),
        )
    }
}

/// Parses a bundle file into its messages, in file order, and everything wrong with it.
pub fn parse(source: &str) -> (Vec<RawMessage>, Vec<FileError>) {
    let (table, toml_errors) = DeTable::parse_recoverable(source);
    let lines = LineIndex::new(source);
    let mut errors: Vec<FileError> = toml_errors
        .iter()
        .map(|error| {
            let (line, column) = error.span().map_or((1, 1), |span| lines.position(span.start));
            FileError {
                line,
                column,
                reason: error.message().to_owned(),
                message: None,
                duplicate: error.message().contains("duplicate key"),
            }
        })
        .collect();
    let mut messages = Vec::new();
    for (key, value) in table.get_ref() {
        let (line, column) = lines.position(key.span().start);
        let id = key.get_ref().to_string();
        if let Some(reason) = invalid_id(&id) {
            errors.push(FileError {
                line,
                column,
                reason,
                message: Some(id),
                duplicate: false,
            });
            continue;
        }
        let DeValue::String(pattern) = value.get_ref() else {
            let (line, column) = lines.position(value.span().start);
            errors.push(FileError {
                line,
                column,
                reason: format!(
                    "message '{id}' must be a string pattern, not {}; a bundle file is one flat table of id = \"pattern\"",
                    value.get_ref().type_str()
                ),
                message: Some(id),
                duplicate: false,
            });
            continue;
        };
        let pattern = pattern.to_string();
        let span = value.span();
        let pattern_offset = source
            .get(span.clone())
            .and_then(|raw| raw.find(&pattern))
            .map(|index| span.start + index);
        messages.push(RawMessage {
            id,
            pattern,
            line,
            column,
            pattern_offset,
        });
    }
    messages.sort_by_key(|message| (message.line, message.column));
    errors.sort_by_key(|error| (error.line, error.column));
    (messages, errors)
}

/// Why a string can't be a message id, if it can't.
pub fn invalid_id(id: &str) -> Option<String> {
    if id.is_empty() {
        return Some("a message id can't be empty".to_owned());
    }
    if id.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Some(format!("message id '{}' has whitespace in it", id.escape_debug()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_keep_their_lines() {
        let source = "# greeting\nhello = \"Hello, {name}!\"\n\nfile_count = '''\n{count, plural,\n  one {# file}\n  other {# files}}'''\n";
        let (messages, errors) = parse(source);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(messages.len(), 2);
        assert_eq!(
            (messages[0].id.as_str(), messages[0].line, messages[0].column),
            ("hello", 2, 1)
        );
        assert_eq!((messages[1].id.as_str(), messages[1].line), ("file_count", 4));
        assert_eq!(messages[1].pattern, "{count, plural,\n  one {# file}\n  other {# files}}");
        let offset = messages[1].pattern_offset.unwrap();
        assert_eq!(LineIndex::new(source).position(offset), (5, 1));
    }

    #[test]
    fn escapes_lose_the_exact_offset_but_not_the_pattern() {
        let (messages, errors) = parse("tab = \"a\\tb {x}\"\n");
        assert_eq!(errors, []);
        assert_eq!(messages[0].pattern, "a\tb {x}");
        assert_eq!(messages[0].pattern_offset, None);
    }

    #[test]
    fn duplicate_ids_are_errors() {
        let (messages, errors) = parse("hello = 'a'\nbye = 'b'\nhello = 'c'\n");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].duplicate, "{errors:?}");
        assert_eq!(errors[0].line, 3);
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn syntax_errors_are_placed() {
        let (_, errors) = parse("hello = 'a'\nbroken = \n");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].line, 2);
        assert!(!errors[0].duplicate);
    }

    #[test]
    fn only_strings_are_messages() {
        let (messages, errors) = parse("count = 3\n[section]\nnested = 'x'\n");
        assert_eq!(messages, []);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(
            errors[0].reason.contains("must be a string pattern, not integer"),
            "{errors:?}"
        );
        assert!(errors[1].reason.contains("not table"), "{errors:?}");
        let (_, errors) = parse("'two words' = 'x'\n");
        assert!(errors[0].reason.contains("whitespace"), "{errors:?}");
    }

    #[test]
    fn positions_count_characters() {
        let lines = LineIndex::new("ab\ncd\n\nx");
        assert_eq!(lines.position(0), (1, 1));
        assert_eq!(lines.position(2), (1, 3));
        assert_eq!(lines.position(3), (2, 1));
        assert_eq!(lines.position(4), (2, 2));
        assert_eq!(lines.position(6), (3, 1));
        assert_eq!(lines.position(7), (4, 1));
        assert_eq!(lines.position(99), (4, 2));
        assert_eq!(LineIndex::new("é = 'x'").position(3), (1, 3));
    }
}

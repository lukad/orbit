use std::{borrow::Cow, path::Path};

use rustyline::{
    Config, Result,
    history::{DefaultHistory, History, SearchDirection, SearchResult},
};

// Rustyline lays out embedded newlines from column zero, so the padding has
// to be part of the recalled buffer. The zero-width marker makes that padding
// reversible without adding whitespace to saved history or executed source.
pub(super) const ALIGNMENT_MARKER: char = '\u{034f}';
const PADDED_NEWLINE: &str = "\n  \u{034f}";

pub(super) struct ReplHistory {
    inner: DefaultHistory,
}

impl ReplHistory {
    pub(super) fn with_config(config: &Config) -> Self {
        Self {
            inner: DefaultHistory::with_config(config),
        }
    }
}

pub(super) fn decode_entry(mut entry: String) -> String {
    if entry.contains(PADDED_NEWLINE) {
        entry = entry.replace(PADDED_NEWLINE, "\n");
    }
    entry
}

impl History for ReplHistory {
    fn get(&self, index: usize, direction: SearchDirection) -> Result<Option<SearchResult<'_>>> {
        self.inner
            .get(index, direction)
            .map(|result| result.map(align_result))
    }

    fn add(&mut self, line: &str) -> Result<bool> {
        self.inner.add(line)
    }

    fn add_owned(&mut self, line: String) -> Result<bool> {
        self.inner.add_owned(line)
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    fn set_max_len(&mut self, len: usize) -> Result<()> {
        self.inner.set_max_len(len)
    }

    fn ignore_dups(&mut self, yes: bool) -> Result<()> {
        self.inner.ignore_dups(yes)
    }

    fn ignore_space(&mut self, yes: bool) {
        self.inner.ignore_space(yes);
    }

    fn save(&mut self, path: &Path) -> Result<()> {
        self.inner.save(path)
    }

    fn append(&mut self, path: &Path) -> Result<()> {
        self.inner.append(path)
    }

    fn load(&mut self, path: &Path) -> Result<()> {
        self.inner.load(path)
    }

    fn clear(&mut self) -> Result<()> {
        self.inner.clear()
    }

    fn search(
        &self,
        term: &str,
        start: usize,
        direction: SearchDirection,
    ) -> Result<Option<SearchResult<'_>>> {
        let term = decode_term(term);
        self.inner
            .search(&term, start, direction)
            .map(|result| result.map(align_result))
    }

    fn starts_with(
        &self,
        term: &str,
        start: usize,
        direction: SearchDirection,
    ) -> Result<Option<SearchResult<'_>>> {
        let term = decode_term(term);
        self.inner
            .starts_with(&term, start, direction)
            .map(|result| result.map(align_result))
    }
}

fn align_result<'entry>(result: SearchResult<'entry>) -> SearchResult<'entry> {
    if !result.entry.contains('\n') {
        return result;
    }

    let position = result.pos
        + result.entry[..result.pos]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            * (PADDED_NEWLINE.len() - 1);
    SearchResult {
        entry: Cow::Owned(result.entry.replace('\n', PADDED_NEWLINE)),
        idx: result.idx,
        pos: position,
    }
}

fn decode_term(term: &str) -> Cow<'_, str> {
    if term.contains(PADDED_NEWLINE) {
        Cow::Owned(term.replace(PADDED_NEWLINE, "\n"))
    } else {
        Cow::Borrowed(term)
    }
}

#[cfg(test)]
mod tests {
    use rustyline::{
        Config,
        history::{History, SearchDirection},
    };

    use super::{ALIGNMENT_MARKER, ReplHistory, decode_entry};

    #[test]
    fn recalled_multiline_entries_are_aligned_with_the_prompt() {
        let mut history = ReplHistory::with_config(&Config::default());
        let source = "function div(a, b)\n  return a / b\nend";
        history.add(source).unwrap();

        let recalled = history
            .get(0, SearchDirection::Reverse)
            .unwrap()
            .unwrap()
            .entry
            .into_owned();

        assert_eq!(
            recalled,
            format!(
                "function div(a, b)\n  {ALIGNMENT_MARKER}  return a / b\n  {ALIGNMENT_MARKER}end"
            )
        );
        assert_eq!(decode_entry(recalled), source);
    }

    #[test]
    fn decoding_does_not_change_regular_multiline_input() {
        let source = "message = [[first\n  second]]".to_owned();

        assert_eq!(decode_entry(source.clone()), source);
    }

    #[test]
    fn searches_return_aligned_entries_and_adjusted_positions() {
        let mut history = ReplHistory::with_config(&Config::default());
        history.add("first\nsecond needle").unwrap();

        let result = history
            .search("needle", 0, SearchDirection::Forward)
            .unwrap()
            .unwrap();

        assert_eq!(
            &result.entry[result.pos..],
            "needle",
            "the cursor position should account for display padding"
        );
        assert_eq!(
            decode_entry(result.entry.into_owned()),
            "first\nsecond needle"
        );
    }
}

use orbit_vm::VmError;

use crate::error;

const ESC: u8 = b'%';
const MAX_CAPTURES: usize = 32;
const MAX_MATCH_DEPTH: usize = 200;

#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum PatternError {
    #[error("malformed pattern (ends with '%')")]
    EndsWithEscape,
    #[error("malformed pattern (missing ']')")]
    MissingBracket,
    #[error("invalid pattern capture")]
    MissingCloseParen,
    #[error("unfinished capture")]
    UnfinishedCapture,
    #[error("invalid capture index")]
    InvalidCaptureIndex,
    #[error("too many captures")]
    TooManyCaptures,
    #[error("missing arguments to '%b' in pattern")]
    MissingBalanceArgs,
    #[error("missing '[' after '%f' in pattern")]
    MissingFrontierSet,
    #[error("pattern too complex")]
    TooComplex,
}

impl From<PatternError> for VmError {
    fn from(value: PatternError) -> Self {
        error::failure(value.to_string())
    }
}

#[derive(Clone, Copy)]
enum Capture {
    Unfinished { start: usize },
    Position { start: usize },
    Closed { start: usize, len: usize },
}

pub(crate) enum CaptureValue {
    Text { start: usize, end: usize },
    Position(usize),
}

pub(crate) struct Match {
    pub start: usize,
    pub end: usize,
    pub captures: Vec<CaptureValue>,
}

struct Matcher<'a> {
    subject: &'a [u8],
    pattern: &'a [u8],
    captures: [Capture; MAX_CAPTURES],
    level: usize,
    depth: usize,
}

impl<'a> Matcher<'a> {
    /// Index one past the current pattern item (char, %x, or [set])
    fn class_end(&self, pat: usize) -> Result<usize, PatternError> {
        let pattern = self.pattern;
        match pattern[pat] {
            ESC => {
                if pat + 1 == pattern.len() {
                    return Err(PatternError::EndsWithEscape);
                }
                Ok(pat + 2)
            }
            b'[' => {
                let mut i = pat + 1;
                if pattern.get(i) == Some(&b'^') {
                    i += 1;
                }
                loop {
                    if i == pattern.len() {
                        return Err(PatternError::MissingBracket);
                    }

                    let c = pattern[i];
                    i += 1;

                    if c == ESC && i < pattern.len() {
                        i += 1;
                    }

                    if pattern.get(i) == Some(&b']') {
                        return Ok(i + 1);
                    }
                }
            }
            _ => Ok(pat + 1),
        }
    }

    /// Matches a byte `c` against a `class` where `class` is the byte after '%'.
    /// Uppercase class letters invert the result.
    fn match_class(c: u8, class: u8) -> bool {
        let result = match class.to_ascii_lowercase() {
            b'a' => c.is_ascii_alphabetic(),
            b'c' => c.is_ascii_control(),
            b'd' => c.is_ascii_digit(),
            b'g' => c.is_ascii_graphic(),
            b'l' => c.is_ascii_lowercase(),
            b'p' => c.is_ascii_punctuation(),
            b's' => matches!(c, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r'),
            b'u' => c.is_ascii_uppercase(),
            b'w' => c.is_ascii_alphanumeric(),
            b'x' => c.is_ascii_hexdigit(),
            _ => return c == class,
        };
        if class.is_ascii_uppercase() {
            !result
        } else {
            result
        }
    }

    /// `pat` points at '[', `set_end` at the closing ']'.
    fn match_bracket_class(&self, c: u8, pat: usize, set_end: usize) -> bool {
        let pattern = self.pattern;
        let mut matched = true;
        let mut i = pat + 1;

        if pattern[i] == b'^' {
            matched = false;
            i += 1;
        }

        while i < set_end {
            if pattern[i] == ESC {
                if Self::match_class(c, pattern[i + 1]) {
                    return matched;
                }
                i += 2;
            } else if pattern.get(i + 1) == Some(&b'-') && i + 2 < set_end {
                if pattern[i] <= c && c <= pattern[i + 2] {
                    return matched;
                }
                i += 3;
            } else {
                if pattern[i] == c {
                    return matched;
                }
                i += 1;
            }
        }

        !matched
    }

    /// Does pattern item at `pat` (ending at `item_end`) match subject byte at `subj`?
    fn single_match(&self, subj: usize, pat: usize, item_end: usize) -> bool {
        let Some(&c) = self.subject.get(subj) else {
            return false;
        };

        match self.pattern[pat] {
            b'.' => true,
            ESC => Self::match_class(c, self.pattern[pat + 1]),
            b'[' => self.match_bracket_class(c, pat, item_end - 1),
            literal => literal == c,
        }
    }

    /// `%bxy` at pattern index `pat` (pat points at x).
    fn match_balance(&self, mut subj: usize, pat: usize) -> Result<Option<usize>, PatternError> {
        if pat + 1 >= self.pattern.len() {
            return Err(PatternError::MissingBalanceArgs);
        }

        let (b, e) = (self.pattern[pat], self.pattern[pat + 1]);

        if self.subject.get(subj) != Some(&b) {
            return Ok(None);
        }

        let mut depth = 1usize;

        loop {
            subj += 1;
            match self.subject.get(subj) {
                None => return Ok(None),
                Some(&c) if c == e => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(Some(subj + 1));
                    }
                }
                Some(&c) if c == b => depth += 1,
                _ => {}
            }
        }
    }

    /// Greedy: longest first, backtrack down.
    fn max_expand(
        &mut self,
        subj: usize,
        pat: usize,
        item_end: usize,
    ) -> Result<Option<usize>, PatternError> {
        let mut i = 0;

        while self.single_match(subj + i, pat, item_end) {
            i += 1;
        }

        loop {
            if let Some(end) = self.do_match(subj + i, item_end + 1)? {
                return Ok(Some(end));
            }
            if i == 0 {
                return Ok(None);
            }
            i -= 1;
        }
    }

    /// Lazy (`-`): shortest first, grow.
    fn min_expand(
        &mut self,
        mut subj: usize,
        pat: usize,
        item_end: usize,
    ) -> Result<Option<usize>, PatternError> {
        loop {
            if let Some(end) = self.do_match(subj, item_end + 1)? {
                return Ok(Some(end));
            }
            if self.single_match(subj, pat, item_end) {
                subj += 1;
            } else {
                return Ok(None);
            }
        }
    }

    fn start_capture(
        &mut self,
        subj: usize,
        pat: usize,
        position: bool,
    ) -> Result<Option<usize>, PatternError> {
        if self.level == MAX_CAPTURES {
            return Err(PatternError::TooManyCaptures);
        }

        self.captures[self.level] = if position {
            Capture::Position { start: subj }
        } else {
            Capture::Unfinished { start: subj }
        };

        self.level += 1;

        match self.do_match(subj, pat)? {
            some @ Some(_) => Ok(some),
            None => {
                self.level -= 1;
                Ok(None)
            }
        }
    }

    fn end_capture(&mut self, subj: usize, pat: usize) -> Result<Option<usize>, PatternError> {
        let Some(idx) = (0..self.level)
            .rev()
            .find(|&idx| matches!(self.captures[idx], Capture::Unfinished { .. }))
        else {
            return Err(PatternError::MissingCloseParen);
        };

        let Capture::Unfinished { start } = self.captures[idx] else {
            unreachable!()
        };

        self.captures[idx] = Capture::Closed {
            start,
            len: subj - start,
        };

        match self.do_match(subj, pat)? {
            some @ Some(_) => Ok(some),
            None => {
                self.captures[idx] = Capture::Unfinished { start }; // undo
                Ok(None)
            }
        }
    }

    /// Back-reference `%1`-`%9` (`%0` errors: digit - b'1' underflows the check).
    fn match_capture(&self, subj: usize, digit: u8) -> Result<Option<usize>, PatternError> {
        let idx = digit.wrapping_sub(b'1') as usize;
        if idx >= self.level {
            return Err(PatternError::InvalidCaptureIndex);
        }

        let Capture::Closed { start, len } = self.captures[idx] else {
            return Err(PatternError::InvalidCaptureIndex); // capture still open
        };

        if self.subject.len() - subj >= len
            && self.subject[start..start + len] == self.subject[subj..subj + len]
        {
            Ok(Some(subj + len))
        } else {
            Ok(None)
        }
    }

    fn do_match(&mut self, subj: usize, pat: usize) -> Result<Option<usize>, PatternError> {
        if self.depth == 0 {
            return Err(PatternError::TooComplex);
        }
        self.depth -= 1;
        let result = self.match_at(subj, pat);
        self.depth += 1;
        result
    }

    fn match_at(&mut self, mut subj: usize, mut pat: usize) -> Result<Option<usize>, PatternError> {
        while pat < self.pattern.len() {
            match self.pattern[pat] {
                b'(' => {
                    let (position, next) = if self.pattern.get(pat + 1) == Some(&b')') {
                        (true, pat + 2)
                    } else {
                        (false, pat + 1)
                    };
                    return self.start_capture(subj, next, position);
                }
                b')' => return self.end_capture(subj, pat + 1),
                b'$' if pat + 1 == self.pattern.len() => {
                    return Ok((subj == self.subject.len()).then_some(subj));
                }
                ESC => match self.pattern.get(pat + 1) {
                    Some(b'b') => {
                        subj = match self.match_balance(subj, pat + 2)? {
                            Some(end) => end,
                            None => return Ok(None),
                        };
                        pat += 4;
                        continue;
                    }
                    Some(b'f') => {
                        pat += 2;
                        if self.pattern.get(pat) != Some(&b'[') {
                            return Err(PatternError::MissingFrontierSet);
                        }
                        let item_end = self.class_end(pat)?;
                        let previous = if subj == 0 { 0 } else { self.subject[subj - 1] };
                        let current = self.subject.get(subj).copied().unwrap_or(0);

                        if !self.match_bracket_class(previous, pat, item_end - 1)
                            && self.match_bracket_class(current, pat, item_end - 1)
                        {
                            pat = item_end;
                            continue;
                        }

                        return Ok(None);
                    }
                    Some(&d @ b'0'..=b'9') => {
                        subj = match self.match_capture(subj, d)? {
                            Some(end) => end,
                            None => return Ok(None),
                        };
                        pat += 2;
                        continue;
                    }
                    _ => (),
                },
                _ => (),
            }

            // Single item plus optional suffix — the C `dflt:` case.
            let item_end = self.class_end(pat)?;
            if self.single_match(subj, pat, item_end) {
                match self.pattern.get(item_end) {
                    Some(b'?') => {
                        if let some @ Some(_) = self.do_match(subj + 1, item_end + 1)? {
                            return Ok(some);
                        }
                        pat = item_end + 1;
                    }
                    Some(b'+') => return self.max_expand(subj + 1, pat, item_end),
                    Some(b'*') => return self.max_expand(subj, pat, item_end),
                    Some(b'-') => return self.min_expand(subj, pat, item_end),
                    _ => {
                        subj += 1;
                        pat = item_end;
                    }
                }
            } else {
                match self.pattern.get(item_end) {
                    Some(b'*') | Some(b'?') | Some(b'-') => pat = item_end + 1,
                    _ => return Ok(None),
                }
            }
        }

        Ok(Some(subj))
    }

    fn collect(&self) -> Result<Vec<CaptureValue>, PatternError> {
        self.captures[..self.level]
            .iter()
            .map(|capture| match *capture {
                Capture::Position { start } => Ok(CaptureValue::Position(start + 1)),
                Capture::Closed { start, len } => Ok(CaptureValue::Text {
                    start,
                    end: start + len,
                }),
                Capture::Unfinished { .. } => Err(PatternError::UnfinishedCapture),
            })
            .collect()
    }
}

pub(crate) fn find(
    subject: &[u8],
    pattern: &[u8],
    start: usize, // 0-based, from start_offset
) -> Result<Option<Match>, PatternError> {
    let anchor = pattern.first() == Some(&b'^');
    let pattern = if anchor { &pattern[1..] } else { pattern };

    let mut s = start;

    loop {
        if let Some(matched) = match_at(subject, pattern, s)? {
            return Ok(Some(matched));
        }

        if anchor || s == subject.len() {
            return Ok(None);
        }

        s += 1;
    }
}

pub(crate) fn match_at(
    subject: &[u8],
    pattern: &[u8],
    start: usize,
) -> Result<Option<Match>, PatternError> {
    if start > subject.len() {
        return Ok(None);
    }

    let mut matcher = Matcher {
        subject,
        pattern,
        captures: [Capture::Closed { start: 0, len: 0 }; MAX_CAPTURES],
        level: 0,
        depth: MAX_MATCH_DEPTH,
    };

    let Some(end) = matcher.do_match(start, 0)? else {
        return Ok(None);
    };

    Ok(Some(Match {
        start,
        end,
        captures: matcher.collect()?,
    }))
}

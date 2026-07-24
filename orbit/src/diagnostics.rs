use std::{cell::RefCell, collections::HashMap, rc::Rc};

use ariadne::{Color, Config, IndexType, Label, Report, ReportKind, Source};
use orbit_common::{SourceId, Span};
use orbit_vm::{LoadError, VmError, VmErrorKind, VmTraceFrame};

pub(crate) type SharedSources = Rc<RefCell<SourceMap>>;

pub(crate) fn shared_sources() -> SharedSources {
    Rc::new(RefCell::new(SourceMap::default()))
}

#[derive(Default)]
pub(crate) struct SourceMap {
    entries: HashMap<SourceId, SourceRecord>,
}

struct SourceRecord {
    name: String,
    text: String,
    byte_offsets: Option<Box<[usize]>>,
}

impl SourceMap {
    pub(crate) fn insert(&mut self, source_id: SourceId, name: &[u8], source: Vec<u8>) {
        let (text, byte_offsets) = match String::from_utf8(source) {
            Ok(text) => (text, None),
            Err(error) => {
                let (text, byte_offsets) = escape_invalid_utf8(&error.into_bytes());
                (text, Some(byte_offsets))
            }
        };

        self.entries.insert(
            source_id,
            SourceRecord {
                name: String::from_utf8_lossy(name).into_owned(),
                text,
                byte_offsets,
            },
        );
    }

    fn get(&self, source_id: SourceId) -> Option<&SourceRecord> {
        self.entries.get(&source_id)
    }
}

impl SourceRecord {
    fn range(&self, span: Span) -> std::ops::Range<usize> {
        let start = usize::try_from(span.start).unwrap_or(usize::MAX);
        let end = usize::try_from(span.end).unwrap_or(usize::MAX);

        let (start, end) = match &self.byte_offsets {
            Some(offsets) => {
                let last = offsets.len() - 1;
                (offsets[start.min(last)], offsets[end.min(last)])
            }
            None => (start.min(self.text.len()), end.min(self.text.len())),
        };

        start.min(end)..start.max(end)
    }
}

fn escape_invalid_utf8(source: &[u8]) -> (String, Box<[usize]>) {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut text = String::new();
    let mut byte_offsets = vec![0; source.len() + 1];
    let mut position = 0;

    while position < source.len() {
        let (valid_length, invalid_length) = match std::str::from_utf8(&source[position..]) {
            Ok(valid) => (valid.len(), 0),
            Err(error) => (
                error.valid_up_to(),
                error
                    .error_len()
                    .unwrap_or(source.len() - position - error.valid_up_to()),
            ),
        };

        if valid_length > 0 {
            let output_start = text.len();
            let valid = std::str::from_utf8(&source[position..position + valid_length])
                .expect("Utf8Error::valid_up_to identifies valid UTF-8");
            text.push_str(valid);

            for offset in 0..=valid_length {
                byte_offsets[position + offset] = output_start + offset;
            }

            position += valid_length;
        }

        for _ in 0..invalid_length {
            let byte = source[position];
            byte_offsets[position] = text.len();
            text.push('\\');
            text.push('x');
            text.push(HEX[usize::from(byte >> 4)] as char);
            text.push(HEX[usize::from(byte & 0x0f)] as char);
            position += 1;
            byte_offsets[position] = text.len();
        }
    }

    (text, byte_offsets.into_boxed_slice())
}

pub(crate) fn print_runtime_error(error: &VmError, sources: &SourceMap) {
    match &error.kind {
        VmErrorKind::LoadFailure(error) => print_load_error(error, sources),
        kind => {
            let span = error.frames.iter().find_map(|frame| match frame {
                VmTraceFrame::Lua {
                    function_span,
                    instruction_span,
                    ..
                } => Some(instruction_span.unwrap_or(*function_span)),
                VmTraceFrame::Native { .. } => None,
            });

            if span.is_none_or(|span| !print_diagnostic(span, kind, sources)) {
                eprintln!("{kind}");
            }
        }
    }

    if error.frames.is_empty() {
        return;
    }

    eprintln!("stack traceback:");

    for frame in &error.frames {
        match frame {
            VmTraceFrame::Lua {
                function_span,
                pc,
                instruction_span,
                ..
            } => {
                let span = instruction_span.unwrap_or(*function_span);

                if let Some(source) = sources.get(span.source) {
                    let (line, column) = line_column(&source.text, span.start);
                    eprintln!("\t{}:{line}:{column} (pc {pc})", source.name);
                } else {
                    eprintln!(
                        "\t[source {} bytes {}..{}, pc {}]",
                        span.source.get(),
                        span.start,
                        span.end,
                        pc,
                    );
                }
            }
            VmTraceFrame::Native { name } => {
                eprintln!("\t[native: {name}]");
            }
        }
    }
}

fn print_load_error(error: &LoadError, sources: &SourceMap) {
    match error {
        LoadError::InvalidUtf8 { span } => print_load_diagnostic(*span, error, sources),
        LoadError::Lex(error) => print_load_diagnostic(error.span, error, sources),
        LoadError::Parse(error) => print_load_diagnostic(error.span, error, sources),
        LoadError::Resolve { diagnostics } => {
            for diagnostic in diagnostics {
                print_load_diagnostic(diagnostic.span, diagnostic, sources);
            }
        }
        LoadError::Compile(error) => print_load_diagnostic(error.span, error, sources),
        error => match error.source_id() {
            Some(source_id) => eprintln!("[source {}]: {error}", source_id.get()),
            None => eprintln!("{error}"),
        },
    }
}

fn print_load_diagnostic(span: Span, message: impl std::fmt::Display, sources: &SourceMap) {
    if !print_diagnostic(span, &message, sources) {
        eprintln!(
            "[source {} bytes {}..{}]: {message}",
            span.source.get(),
            span.start,
            span.end,
        );
    }
}

fn print_diagnostic(span: Span, message: impl std::fmt::Display, sources: &SourceMap) -> bool {
    let Some(source) = sources.get(span.source) else {
        return false;
    };
    let range = source.range(span);
    let report_span = (source.name.as_str(), range.clone());
    let report = Report::build(ReportKind::Error, report_span)
        .with_config(Config::default().with_index_type(IndexType::Byte))
        .with_message(message.to_string())
        .with_label(
            Label::new((source.name.as_str(), range))
                .with_message("here")
                .with_color(Color::Red),
        )
        .finish();

    report
        .eprint((source.name.as_str(), Source::from(source.text.as_str())))
        .is_ok()
}

fn line_column(source: &str, byte_offset: u32) -> (usize, usize) {
    let mut offset = usize::try_from(byte_offset)
        .unwrap_or(usize::MAX)
        .min(source.len());

    while !source.is_char_boundary(offset) {
        offset -= 1;
    }

    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count()
        + 1;

    (line, column)
}

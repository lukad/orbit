use std::{path::Path, process::ExitCode};

use orbit_common::SourceId;
use orbit_vm::{Environment, VmError, VmTraceFrame};

fn main() -> ExitCode {
    let path = std::env::args_os()
        .nth(1)
        .expect("Expected a path argument");
    let source = std::fs::read_to_string(&path).expect("Failed to read file");
    let source_id = SourceId::new(0);
    let tokens = orbit_parser::lexer::lex(source_id, &source).unwrap();
    let ast = orbit_parser::parser::parse_chunk(source_id, &tokens).unwrap();
    let hir = orbit_resolver::resolve(&ast).unwrap();
    let compiled = orbit_compiler::compile(hir).unwrap();
    let environment = Environment::new();
    match orbit_vm::execute(&compiled, &environment) {
        Ok(values) => {
            println!("{values:?}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            print_runtime_error(Path::new(&path), &source, &error);
            ExitCode::FAILURE
        }
    }
}

fn print_runtime_error(path: &Path, source: &str, error: &VmError) {
    eprintln!("{}", error.kind);

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
            } => {
                let span = instruction_span.unwrap_or(*function_span);
                let (line, column) = line_column(source, span.start);
                eprintln!("\t{}:{line}:{column} (pc {pc})", path.display());
            }
            VmTraceFrame::Native { name } => eprintln!("\t[native: {name}]"),
        }
    }
}

fn line_column(source: &str, byte_offset: u32) -> (usize, usize) {
    let mut offset = usize::try_from(byte_offset).unwrap_or(usize::MAX);
    offset = offset.min(source.len());

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

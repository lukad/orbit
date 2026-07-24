mod diagnostics;
mod repl;
mod source_loader;

use std::{borrow::Cow, ffi::OsStr, process::ExitCode};

use orbit_vm::{CallOutcome, LoadSource, State};

use crate::{
    diagnostics::{SharedSources, print_runtime_error, shared_sources},
    source_loader::DiagnosticLoader,
};

fn main() -> ExitCode {
    let filename = std::env::args_os().nth(1);
    let filename = match filename.as_deref() {
        Some(filename) => {
            let Some(filename) = filename_bytes(filename) else {
                eprintln!("filename cannot be represented on this platform");
                return ExitCode::FAILURE;
            };
            Some(filename)
        }
        None => None,
    };

    let sources = shared_sources();
    let loader = DiagnosticLoader::new(SharedSources::clone(&sources));
    let mut state = match State::new(loader) {
        Ok(state) => state,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = orbit_stdlib::install(&mut state) {
        print_runtime_error(&error, &sources.borrow());
        return ExitCode::FAILURE;
    }

    match filename {
        Some(filename) => run_file(&mut state, &sources, &filename),
        None => repl::run(&mut state, &sources),
    }
}

fn run_file(state: &mut State, sources: &SharedSources, filename: &[u8]) -> ExitCode {
    let main = match state.load_source(LoadSource::File { filename }) {
        Ok(main) => main,
        Err(error) => {
            print_runtime_error(&error, &sources.borrow());
            return ExitCode::FAILURE;
        }
    };

    match state.call(&main, &[]) {
        Ok(CallOutcome::Returned(_)) => ExitCode::SUCCESS,
        Ok(CallOutcome::Yielded { .. }) => {
            eprintln!("main chunk unexpectedly yielded");
            ExitCode::FAILURE
        }
        Err(error) => {
            print_runtime_error(&error, &sources.borrow());
            ExitCode::FAILURE
        }
    }
}

#[cfg(unix)]
fn filename_bytes(filename: &OsStr) -> Option<Cow<'_, [u8]>> {
    use std::os::unix::ffi::OsStrExt;

    Some(Cow::Borrowed(filename.as_bytes()))
}

#[cfg(not(unix))]
fn filename_bytes(filename: &OsStr) -> Option<Cow<'_, [u8]>> {
    filename
        .to_str()
        .map(|filename| Cow::Borrowed(filename.as_bytes()))
}

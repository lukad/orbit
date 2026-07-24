//! Harness for the upstream Lua 5.4 test suite in `vendor/lua/testes`.
//!
//! The normal test run compiles every suite source and checks the known-gap
//! baseline. Runtime conformance cases are ignored by default because most do
//! not pass yet. Run all of them with:
//!
//! `cargo test -p orbit --test lua54_suite -- --ignored --nocapture --test-threads=1`
//!
//! Each Lua file is a separate Rust test, so normal Cargo filtering works:
//!
//! `cargo test -p orbit --test lua54_suite runtime_vararg -- --ignored --nocapture`

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use orbit_common::SourceId;
use orbit_loader::Loader;
use orbit_vm::{LoadService, LoadSource};

const KNOWN_COMPILE_FAILURES: &[&str] = &[
    "bitwise.lua",
    "code.lua",
    "literals.lua",
    "math.lua",
    "strings.lua",
    "tpack.lua",
    "utf8.lua",
];

const INDIVIDUAL_RUNNER_PRELUDE: &str = r#"
_U = true
_soft = false
_port = true
_nomsg = true
T = nil

function Message (_) end
"#;

static NEXT_RUNNER_ID: AtomicU64 = AtomicU64::new(0);
static RUNTIME_TEST_LOCK: Mutex<()> = Mutex::new(());

fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../vendor/lua/testes")
}

fn suite_files() -> Vec<PathBuf> {
    let directory = suite_dir();
    let entries = fs::read_dir(&directory).unwrap_or_else(|error| {
        panic!(
            "cannot read the Lua 5.4 test suite at {}: {error}\n\
             initialize it with `git submodule update --init vendor/lua`",
            directory.display()
        )
    });

    let mut files = entries
        .map(|entry| {
            entry
                .expect("failed to read a Lua test-suite directory entry")
                .path()
        })
        .filter(|path| path.extension().is_some_and(|extension| extension == "lua"))
        .collect::<Vec<_>>();
    files.sort();

    assert!(
        files.iter().any(|path| path.ends_with("all.lua")),
        "Lua 5.4 test suite is empty or incomplete at {}; initialize it with \
         `git submodule update --init vendor/lua`",
        directory.display()
    );

    files
}

#[test]
fn compiles_upstream_lua54_suite_sources() {
    let directory = suite_dir();
    let mut loader = Loader::new();
    let mut failures = Vec::new();

    for (index, path) in suite_files().iter().enumerate() {
        let relative = path
            .strip_prefix(&directory)
            .expect("suite file should be inside the suite directory");
        let source_id = SourceId::new(u32::try_from(index).expect("suite file index overflow"));

        if let Err(error) = loader.compile(
            source_id,
            LoadSource::File {
                filename: path.as_os_str().as_encoded_bytes(),
            },
        ) {
            failures.push((relative.to_string_lossy().into_owned(), error));
        }
    }

    let actual = failures
        .iter()
        .map(|(path, _)| path.as_str())
        .collect::<BTreeSet<_>>();
    let expected = KNOWN_COMPILE_FAILURES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let unexpected = actual.difference(&expected).copied().collect::<Vec<_>>();
    let repaired = expected.difference(&actual).copied().collect::<Vec<_>>();

    if unexpected.is_empty() && repaired.is_empty() {
        return;
    }

    let details = failures
        .iter()
        .filter(|(path, _)| unexpected.contains(&path.as_str()))
        .map(|(path, error)| format!("  {path}: {error:?}"))
        .collect::<Vec<_>>()
        .join("\n");

    panic!(
        "Lua 5.4 compile baseline changed.\n\
         unexpected failures: {unexpected:?}\n\
         repaired known failures: {repaired:?}\n\
         update KNOWN_COMPILE_FAILURES after reviewing the change.\n{details}"
    );
}

macro_rules! lua54_runtime_tests {
    ($($name:ident => $file:literal),+ $(,)?) => {
        $(
            #[test]
            #[ignore = "upstream Lua 5.4 runtime conformance case"]
            fn $name() {
                run_upstream_file($file);
            }
        )+
    };
}

// `main.lua` exercises the standalone PUC Lua CLI, and `api.lua` exercises
// Lua's private C test module `T`; neither is a language/stdlib conformance
// case for Orbit. `tracegc.lua` and `bwcoercion.lua` are helper modules loaded
// by tests below rather than standalone cases.
lua54_runtime_tests! {
    runtime_gc => "gc.lua",
    runtime_db => "db.lua",
    runtime_calls => "calls.lua",
    runtime_strings => "strings.lua",
    runtime_literals => "literals.lua",
    runtime_tpack => "tpack.lua",
    runtime_attrib => "attrib.lua",
    runtime_gengc => "gengc.lua",
    runtime_locals => "locals.lua",
    runtime_constructs => "constructs.lua",
    runtime_code => "code.lua",
    runtime_big => "big.lua",
    runtime_cstack => "cstack.lua",
    runtime_nextvar => "nextvar.lua",
    runtime_pm => "pm.lua",
    runtime_utf8 => "utf8.lua",
    runtime_events => "events.lua",
    runtime_vararg => "vararg.lua",
    runtime_closure => "closure.lua",
    runtime_coroutine => "coroutine.lua",
    runtime_goto => "goto.lua",
    runtime_errors => "errors.lua",
    runtime_math => "math.lua",
    runtime_sort => "sort.lua",
    runtime_bitwise => "bitwise.lua",
    runtime_verybig => "verybig.lua",
    runtime_files => "files.lua",
}

fn run_upstream_file(test: &str) {
    let directory = suite_dir();
    let runner = TemporaryRunner::new();
    runner.write(test);

    // Several upstream files create shared scratch files in `testes/`. Keep
    // macro-generated cases isolated even when libtest uses multiple threads.
    let output = {
        let _guard = RUNTIME_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Command::new(env!("CARGO_BIN_EXE_orbit"))
            .arg(runner.path())
            .current_dir(&directory)
            .output()
            .unwrap_or_else(|error| panic!("failed to launch Orbit for {test}: {error}"))
    };

    assert!(
        output.status.success(),
        "{test} failed\n\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

struct TemporaryRunner {
    path: PathBuf,
}

impl TemporaryRunner {
    fn new() -> Self {
        let id = NEXT_RUNNER_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "orbit-lua54-runner-{}-{id}.lua",
            std::process::id()
        ));
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, test: &str) {
        let source = format!("{INDIVIDUAL_RUNNER_PRELUDE}\ndofile({test:?})\n");
        fs::write(&self.path, source)
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", self.path.display()));
    }
}

impl Drop for TemporaryRunner {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

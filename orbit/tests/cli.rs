use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_SCRIPT_ID: AtomicU64 = AtomicU64::new(0);

struct Script(PathBuf);

impl Script {
    fn new(source: &str) -> Self {
        let id = NEXT_SCRIPT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("orbit-cli-test-{}-{id}.lua", std::process::id()));
        fs::write(&path, source).unwrap();
        Self(path)
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn runtime_errors_fail_and_include_the_source_location() {
    let script = Script::new("return 1 + true\n");
    let output = Command::new(env!("CARGO_BIN_EXE_orbit"))
        .arg(&script.0)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("attempt to add a number value and a boolean value"));
    assert!(stderr.contains(&format!("{}:1:8 (pc 2)", script.0.display())));
}

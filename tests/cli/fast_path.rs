//! Runtime fast-path integration checks that belong to Jess itself.

use crate::common;

use common::*;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cli/fixtures")
}

fn run_canary(script: &Path, data: &Path) {
    let output = jess().args(["run"]).arg(script).arg(data).arg("1").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}: {}{}",
        script.display(),
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.lines().any(|line| line.starts_with("canary ")),
        "{} printed no time",
        script.display()
    );
}

#[test]
fn builtins_run_on_the_fast_path() {
    run_canary(&fixtures().join("safeenv.jsl"), Path::new("."));
}

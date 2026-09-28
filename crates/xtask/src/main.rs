use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args = env::args().skip(1);
    let Some(task) = args.next() else {
        eprintln!("usage: cargo xtask -- <check-boundaries|fmt|clippy|test|test-nextest|miri|ci>");
        return ExitCode::from(2);
    };
    let result = match task.as_str() {
        "check-boundaries" => check_boundaries(&root),
        "fmt" => cargo(&root, &["fmt", "--all", "--", "--check"]),
        "clippy" => cargo(
            &root,
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        "test" => cargo(&root, &["test", "--workspace"]),
        "test-nextest" => cargo(&root, &["nextest", "run", "--workspace"]),
        "miri" => cargo(&root, &["miri", "test", "-p", "ime-runtime-core"]),
        "ci" => ["check-boundaries", "fmt", "clippy", "test"]
            .into_iter()
            .try_for_each(|name| {
                if name == "check-boundaries" {
                    check_boundaries(&root)
                } else {
                    match name {
                        "fmt" => cargo(&root, &["fmt", "--all", "--", "--check"]),
                        "clippy" => cargo(
                            &root,
                            &[
                                "clippy",
                                "--workspace",
                                "--all-targets",
                                "--",
                                "-D",
                                "warnings",
                            ],
                        ),
                        _ => cargo(&root, &["test", "--workspace"]),
                    }
                }
            }),
        unknown => {
            eprintln!("unknown xtask: {unknown}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask {task} failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn cargo(root: &PathBuf, args: &[&str]) -> Result<(), String> {
    let status = Command::new("cargo")
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {args:?} exited with {status}"))
    }
}

fn check_boundaries(root: &PathBuf) -> Result<(), String> {
    let output = Command::new("cargo")
        .args(["tree", "-p", "ime-runtime-core"])
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("cargo tree failed".into());
    }
    let tree = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    for forbidden in ["windows", "tokio", "rime-sys", "librime"] {
        if tree.lines().any(|line| line.contains(forbidden)) {
            return Err(format!(
                "runtime-core dependency tree contains forbidden `{forbidden}`"
            ));
        }
    }
    for crate_path in ["crates/protocol/src", "crates/runtime-core/src"] {
        let path = root.join(crate_path);
        for entry in std::fs::read_dir(&path).map_err(|e| format!("{}: {e}", path.display()))? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.path().extension().is_some_and(|ext| ext == "rs") {
                let source = std::fs::read_to_string(entry.path()).map_err(|e| e.to_string())?;
                if ["unsafe {", "unsafe fn", "unsafe impl"]
                    .iter()
                    .any(|needle| source.contains(needle))
                {
                    return Err(format!(
                        "unsafe code marker found in {}",
                        entry.path().display()
                    ));
                }
            }
        }
    }
    println!("dependency and unsafe boundaries passed");
    Ok(())
}

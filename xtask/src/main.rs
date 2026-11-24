//! Repository automation.
//!
//! `cargo xtask ci` is the single command that must pass before a commit lands on `dev`.

use std::path::Path;
use std::process::{Command, ExitCode};

/// Crates that must never depend on the query engine.
const NO_ENGINE: &[&str] = &[
    "slickbi-spec",
    "slickbi-calc",
    "slickbi-model",
    "slickbi-planner",
];

/// Crates that are allowed to depend on a graphics stack.
const GPU_ALLOWED: &[&str] = &["slickbi-render-gpu", "slickbi-app", "slickbi-wasm"];

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_else(|| "ci".to_owned());
    let ok = match task.as_str() {
        "ci" => ci(),
        "fmt" => fmt(),
        "clippy" => clippy(),
        "test" => test(),
        "boundaries" => boundaries(),
        other => {
            eprintln!("unknown task '{other}'; expected one of: ci, fmt, clippy, test, boundaries");
            false
        }
    };

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn ci() -> bool {
    fmt() && clippy() && boundaries() && test()
}

fn fmt() -> bool {
    run("cargo", &["fmt", "--all", "--check"])
}

fn clippy() -> bool {
    run(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )
}

fn test() -> bool {
    run("cargo", &["test", "--workspace"])
}

fn run(program: &str, args: &[&str]) -> bool {
    println!("$ {program} {}", args.join(" "));
    match Command::new(program).args(args).status() {
        Ok(status) if status.success() => true,
        Ok(_) => false,
        Err(error) => {
            eprintln!("could not run {program}: {error}");
            false
        }
    }
}

/// Checks the dependency rules in `docs/ARCHITECTURE.md`.
fn boundaries() -> bool {
    let crates_dir = Path::new("crates");
    let entries = match std::fs::read_dir(crates_dir) {
        Ok(entries) => entries,
        Err(error) => {
            eprintln!("boundaries: cannot read {}: {error}", crates_dir.display());
            return false;
        }
    };

    let mut ok = true;
    let mut checked = 0usize;

    for entry in entries.flatten() {
        let manifest = entry.path().join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }

        let Some(name) = package_name(&manifest) else {
            eprintln!(
                "boundaries: cannot read package name from {}",
                manifest.display()
            );
            ok = false;
            continue;
        };

        checked += 1;
        let dependencies = dependencies(&manifest);

        if NO_ENGINE.contains(&name.as_str()) && dependencies.iter().any(|d| d == "slickbi-engine")
        {
            eprintln!("boundary violation: {name} must not depend on slickbi-engine");
            ok = false;
        }

        if !GPU_ALLOWED.contains(&name.as_str()) {
            for forbidden in ["wgpu", "winit", "egui"] {
                if dependencies.iter().any(|d| d == forbidden) {
                    eprintln!("boundary violation: {name} must not depend on {forbidden}");
                    ok = false;
                }
            }
        }
    }

    if ok {
        println!("boundaries: {checked} crates checked");
    }
    ok
}

fn package_name(manifest: &Path) -> Option<String> {
    let document = parse_manifest(manifest)?;
    Some(document.get("package")?.get("name")?.as_str()?.to_owned())
}

fn dependencies(manifest: &Path) -> Vec<String> {
    let Some(document) = parse_manifest(manifest) else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(map) = document.get(table).and_then(|value| value.as_table()) {
            names.extend(map.keys().cloned());
        }
    }
    names
}

fn parse_manifest(manifest: &Path) -> Option<toml::Value> {
    let text = std::fs::read_to_string(manifest).ok()?;
    text.parse::<toml::Value>().ok()
}

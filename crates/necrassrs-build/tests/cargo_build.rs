use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

#[test]
fn cargo_build_compiles_generated_code_and_user_resolver() {
    let directory = create_consumer();
    let build = build_consumer(&directory);
    fs::remove_dir_all(&directory).unwrap();
    assert!(
        build.status.success(),
        "consumer cargo build failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );
}

#[test]
fn sdl_edit_regenerates_contract_without_changing_user_source() {
    let directory = create_consumer();
    let initial = build_consumer(&directory);
    assert!(
        initial.status.success(),
        "initial consumer build failed:\n{}\n{}",
        String::from_utf8_lossy(&initial.stdout),
        String::from_utf8_lossy(&initial.stderr),
    );

    let user_source = fs::read(directory.join("src/main.rs")).unwrap();
    fs::write(
        directory.join("schema/query/fields/hello.graphql"),
        include_str!("fixtures/consumer/hello.graphql").replace("name:", "greeting:"),
    )
    .unwrap();
    let rebuilt = build_consumer(&directory);
    let source_after = fs::read(directory.join("src/main.rs")).unwrap();
    fs::remove_dir_all(&directory).unwrap();

    assert_eq!(user_source, source_after);
    assert!(
        !rebuilt.status.success(),
        "the old argument must no longer compile"
    );
    let stdout = String::from_utf8(rebuilt.stdout).unwrap();
    assert!(
        stdout.lines().any(|line| {
            let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
                return false;
            };
            message["reason"] == "compiler-message" && message["message"]["code"]["code"] == "E0609"
        }),
        "expected E0609 for removed Args.name:\n{stdout}\n{}",
        String::from_utf8_lossy(&rebuilt.stderr),
    );
}

#[test]
fn sdl_addition_regenerates_contract_without_changing_user_source() {
    let directory = create_consumer();
    let initial = build_consumer(&directory);
    let output = generated_path(&initial);
    assert!(!fs::read_to_string(&output).unwrap().contains("addedField"));
    let source = fs::read(directory.join("src/main.rs")).unwrap();

    fs::create_dir_all(directory.join("schema/new/nested")).unwrap();
    fs::write(
        directory.join("schema/new/nested/addition.graphql"),
        "extend type Query { addedField: String! }",
    )
    .unwrap();
    let rebuilt = build_consumer(&directory);
    let regenerated = fs::read_to_string(generated_path(&rebuilt)).unwrap();
    let source_after = fs::read(directory.join("src/main.rs")).unwrap();
    fs::remove_dir_all(&directory).unwrap();

    assert!(regenerated.contains("addedField"));
    assert_eq!(source, source_after);
}

#[test]
fn sdl_deletion_removes_generated_contract_without_changing_user_source() {
    let directory = create_consumer();
    // The library must track SDL even when the consumer tracks other inputs.
    fs::write(
        directory.join("build.rs"),
        include_str!("fixtures/consumer/build.rs").replace(
            "    necrassrs_build::build",
            "    println!(\"cargo::rerun-if-changed=build.rs\");\n    necrassrs_build::build",
        ),
    )
    .unwrap();
    let removed = directory.join("schema/query/fields/removable.graphql");
    fs::write(&removed, "extend type Query { removableField: String! }").unwrap();
    let initial = build_consumer(&directory);
    assert!(
        fs::read_to_string(generated_path(&initial))
            .unwrap()
            .contains("removableField")
    );
    let source = fs::read(directory.join("src/main.rs")).unwrap();

    fs::remove_file(removed).unwrap();
    let rebuilt = build_consumer(&directory);
    let regenerated = fs::read_to_string(generated_path(&rebuilt)).unwrap();
    let source_after = fs::read(directory.join("src/main.rs")).unwrap();
    fs::remove_dir_all(&directory).unwrap();

    assert!(!regenerated.contains("removableField"));
    assert_eq!(source, source_after);
}

fn generated_path(build: &Output) -> PathBuf {
    assert!(
        build.status.success(),
        "consumer build failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );
    String::from_utf8_lossy(&build.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find_map(|message| {
            (message["reason"] == "build-script-executed")
                .then(|| message["out_dir"].as_str().map(PathBuf::from))
                .flatten()
                .map(|directory| directory.join("necrassrs.rs"))
                .filter(|path| path.is_file())
        })
        .expect("Cargo must report the generated file under the consumer OUT_DIR")
}

fn create_consumer() -> PathBuf {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let directory = std::env::temp_dir().join(format!(
        "necrassrs-build-consumer-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir(&directory).unwrap();
    fs::create_dir(directory.join("src")).unwrap();
    fs::create_dir_all(directory.join("schema/query/fields")).unwrap();

    let runtime = workspace.join("crates/necrassrs").canonicalize().unwrap();
    let build_library = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .unwrap();
    let package_name = directory.file_name().unwrap().to_str().unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            r#"
                [package]
                name = "{package_name}"
                version = "0.0.0"
                edition = "2024"
                [workspace]
                [dependencies]
                necrassrs = {{ path = {runtime:?} }}
                [build-dependencies]
                necrassrs-build = {{ path = {build_library:?} }}
                [lints.rust]
                warnings = "deny"
            "#,
        ),
    )
    .unwrap();
    fs::copy(workspace.join("Cargo.lock"), directory.join("Cargo.lock")).unwrap();
    fs::write(
        directory.join("build.rs"),
        include_str!("fixtures/consumer/build.rs"),
    )
    .unwrap();
    fs::write(
        directory.join("schema/schema.graphql"),
        include_str!("fixtures/consumer/schema.graphql"),
    )
    .unwrap();
    fs::write(
        directory.join("schema/query/fields/hello.graphql"),
        include_str!("fixtures/consumer/hello.graphql"),
    )
    .unwrap();
    fs::write(
        directory.join("src/main.rs"),
        include_str!("fixtures/consumer/main.rs"),
    )
    .unwrap();

    directory
}

fn build_consumer(directory: &Path) -> Output {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    Command::new(env!("CARGO"))
        .current_dir(directory)
        .args([
            "build",
            "--offline",
            "--message-format=json",
            "--target-dir",
        ])
        .arg(workspace.join("target/cargo-consumer"))
        .output()
        .expect("Cargo must be available")
}

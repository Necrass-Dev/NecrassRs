use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn help_version_and_invalid_invocations_do_not_create_files() {
    let directory = TestDirectory::new();
    for args in [vec!["--help"], vec!["init", "--help"], vec!["--version"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_necrass"))
            .current_dir(&directory.0)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
    }
    for args in [
        vec![],
        vec!["init", "new-project", "--framework", "unknown"],
        vec!["init", "new-project", "--name", "bad name"],
        vec!["init", "--unknown"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_necrass"))
            .current_dir(&directory.0)
            .args(&args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        if args.is_empty() {
            assert!(stderr.contains("requires a terminal"), "{stderr}");
        }
        assert!(!output.stderr.is_empty());
    }
    assert!(fs::read_dir(&directory.0).unwrap().next().is_none());
}

#[test]
fn init_creates_project_at_explicit_path_and_name() {
    let directory = TestDirectory::new();
    let project = directory.0.join("my-api");
    let output = Command::new(env!("CARGO_BIN_EXE_necrass"))
        .arg("init")
        .arg(&project)
        .args(["--name", "custom-api"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        fs::read_to_string(project.join("Cargo.toml"))
            .unwrap()
            .contains("name = \"custom-api\""),
    );
    for path in [
        "README.md",
        "build.rs",
        "schema/schema.graphql",
        "src/main.rs",
        "src/generated.rs",
        "src/resolvers.rs",
    ] {
        assert!(project.join(path).is_file(), "missing {path}");
    }
    let main = fs::read_to_string(project.join("src/main.rs")).unwrap();
    assert!(main.contains("graphiql_html(\"/graphql\")"));
    assert!(main.contains(".route(\"/graphql\", get("));

    let manifest = fs::read(project.join("Cargo.toml")).unwrap();
    let repeated = Command::new(env!("CARGO_BIN_EXE_necrass"))
        .arg("init")
        .arg(&project)
        .args(["--name", "custom-api"])
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert_eq!(fs::read(project.join("Cargo.toml")).unwrap(), manifest);
}

#[test]
fn generated_consumer_builds_and_rebuilds_without_the_cli() {
    for framework in ["axum", "actix"] {
        check_generated_consumer(framework);
    }
}

fn check_generated_consumer(framework: &str) {
    let directory = TestDirectory::new();
    let project = directory.0.join("consumer");
    let init = Command::new(env!("CARGO_BIN_EXE_necrass"))
        .arg("init")
        .arg(&project)
        .args(["--framework", framework])
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = project.join("Cargo.toml");
    let mut source = fs::read_to_string(&manifest).unwrap();
    let other_framework = if framework == "axum" { "actix" } else { "axum" };
    assert!(!source.contains(&format!("necrassrs-{other_framework}")));
    assert!(source.contains("panic = \"abort\""));
    assert!(!source.contains("rev ="));
    assert!(!source.contains("branch ="));
    assert!(!source.contains("tag ="));
    let adapter = format!("necrassrs-{framework}");
    for package in ["necrassrs", adapter.as_str(), "necrassrs-build"] {
        let git = format!(
            "{package} = {{ git = \"https://github.com/Necrass-Dev/NecrassRs.git\", version = \"0.1.0\" }}"
        );
        let path = workspace
            .join("crates")
            .join(package)
            .canonicalize()
            .unwrap();
        let local = format!("{package} = {{ path = {path:?}, version = \"0.1.0\" }}");
        assert!(
            source.contains(&git),
            "missing Git dependency for {package}"
        );
        source = source.replace(&git, &local);
    }
    let apollo = workspace
        .join("vendor/apollo-compiler")
        .canonicalize()
        .unwrap();
    let patch = r#"apollo-compiler = { git = "https://github.com/Necrass-Dev/NecrassRs.git", version = "1.33.0" }"#;
    assert!(source.contains(patch));
    source = source.replace(patch, &format!("apollo-compiler = {{ path = {apollo:?} }}"));
    fs::write(manifest, source).unwrap();
    fs::copy(workspace.join("Cargo.lock"), project.join("Cargo.lock")).unwrap();

    let build = || {
        Command::new(env!("CARGO"))
            .current_dir(&project)
            .env("CARGO_TERM_COLOR", "never")
            .env("NO_COLOR", "1")
            .args(["build", "--offline", "--target-dir"])
            .arg(workspace.join("target/cargo-consumer"))
            .output()
            .unwrap()
    };
    let first = build();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );

    fs::write(
        project.join("schema/schema.graphql"),
        "type Query { hello(name: String!): String! ping: String! }",
    )
    .unwrap();
    let build_script = project.join("build.rs");
    let mut script = fs::read_to_string(&build_script).unwrap();
    script.push('\n');
    fs::write(&build_script, &script).unwrap();
    let second = build();
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let resolvers = fs::read_to_string(project.join("src/resolvers.rs")).unwrap();
    assert!(resolvers.contains("Hello, {}"));
    assert!(resolvers.contains("async fn r#ping"), "{resolvers}");
    assert!(resolvers.contains("unimplemented!()"));

    fs::write(
        project.join("schema/schema.graphql"),
        "scalar Timestamp\ntype Query { hello(name: Timestamp!): String! }",
    )
    .unwrap();
    script.push('\n');
    fs::write(build_script, script).unwrap();
    let invalid = build();
    assert!(!invalid.status.success());
    let stderr = String::from_utf8_lossy(&invalid.stderr);
    assert!(stderr.contains("schema/schema.graphql"), "{stderr}");
    assert!(stderr.contains("Timestamp!"), "{stderr}");
    assert!(
        !stderr.contains("\u{1b}["),
        "diagnostics must remain readable without color"
    );
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "necrassrs-cli-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

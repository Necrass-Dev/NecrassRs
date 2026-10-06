use std::{
    fs, io,
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use dialoguer::{Input, Select};

#[derive(Parser)]
#[command(
    name = "necrass",
    version,
    about = "Create a NecrassRs GraphQL project"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a project in a new or empty directory.
    Init(InitArgs),
}

#[derive(clap::Args)]
struct InitArgs {
    /// Target directory (defaults to the current directory).
    path: Option<PathBuf>,
    /// Cargo package name (defaults to the directory name).
    #[arg(long, value_parser = validate_name)]
    name: Option<String>,
    /// HTTP framework for the generated server.
    #[arg(long, value_enum, default_value = "axum")]
    framework: Framework,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Framework {
    Axum,
    Actix,
}

fn prepare_target(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_dir() && fs::read_dir(path)?.next().is_none() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("target is not an empty directory: {}", path.display()),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(path)?,
        Err(error) => return Err(error),
    }
    Ok(())
}

fn create_package(path: &Path, name: &str, framework: Framework) -> io::Result<()> {
    let (manifest, main) = match framework {
        Framework::Axum => (
            include_str!("../templates/axum/manifest.toml"),
            include_str!("../templates/axum/main.rs"),
        ),
        Framework::Actix => (
            include_str!("../templates/actix/manifest.toml"),
            include_str!("../templates/actix/main.rs"),
        ),
    };
    prepare_target(path)?;
    fs::create_dir(path.join("src"))?;
    fs::create_dir(path.join("schema"))?;
    for (relative, contents) in [
        ("Cargo.toml", manifest.replace("{{name}}", name)),
        (
            "README.md",
            include_str!("../templates/shared/README.md").to_owned(),
        ),
        (
            "build.rs",
            include_str!("../templates/shared/build.rs").to_owned(),
        ),
        (
            "schema/schema.graphql",
            include_str!("../templates/shared/schema.graphql").to_owned(),
        ),
        ("src/main.rs", main.to_owned()),
        (
            "src/generated.rs",
            include_str!("../templates/shared/generated.rs").to_owned(),
        ),
        (
            "src/resolvers.rs",
            include_str!("../templates/shared/resolvers.rs").to_owned(),
        ),
    ] {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join(relative))?
            .write_all(contents.as_bytes())?;
    }
    Ok(())
}

fn resolve_init(
    init: InitArgs,
    current_dir: &Path,
) -> Result<(PathBuf, String, Framework), String> {
    let path = init.path.unwrap_or_else(|| current_dir.to_path_buf());
    let path = if path == Path::new(".") {
        current_dir.to_path_buf()
    } else {
        path
    };
    let name = init
        .name
        .or_else(|| path.file_name()?.to_str().map(str::to_owned))
        .ok_or("Could not derive a project name from the target directory.")?;
    Ok((path, validate_name(&name)?, init.framework))
}

fn validate_name(name: &str) -> Result<String, String> {
    let mut characters = name.chars();
    if !characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        || !characters.all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return Err("Project name must start with a letter or underscore and contain only ASCII letters, digits, hyphens, or underscores.".into());
    }
    Ok(name.to_owned())
}

fn interactive_init(current_dir: &Path) -> Result<InitArgs, String> {
    let path: String = Input::new()
        .with_prompt("Project directory")
        .default("my-api".into())
        .interact_text()
        .map_err(|error| error.to_string())?;
    let path = PathBuf::from(path);
    let name_path = if path == Path::new(".") {
        current_dir
    } else {
        &path
    };
    let mut prompt = Input::<String>::new()
        .with_prompt("Package name")
        .validate_with(|value: &String| validate_name(value).map(|_| ()));
    if let Some(name) = name_path.file_name().and_then(|name| name.to_str()) {
        prompt = prompt.default(name.to_owned());
    }
    let name = prompt.interact_text().map_err(|error| error.to_string())?;
    let frameworks = Framework::value_variants();
    let choices: Vec<_> = frameworks
        .iter()
        .map(|framework| framework.to_possible_value().unwrap().get_name().to_owned())
        .collect();
    let selected = Select::new()
        .with_prompt("HTTP framework")
        .items(&choices)
        .default(0)
        .interact_opt()
        .map_err(|error| error.to_string())?
        .ok_or("Initialization cancelled.")?;
    Ok(InitArgs {
        path: Some(path),
        name: Some(name),
        framework: frameworks[selected],
    })
}

fn run(cli: Cli) -> Result<(), String> {
    if cli.command.is_none() && !(io::stdin().is_terminal() && io::stderr().is_terminal()) {
        Cli::command()
            .print_help()
            .map_err(|error| error.to_string())?;
        return Err("Interactive initialization requires a terminal. Use necrass init [PATH] [--name NAME] [--framework axum|actix].".into());
    }
    let current_dir = std::env::current_dir().map_err(|error| error.to_string())?;
    let init = match cli.command {
        Some(Commands::Init(init)) => init,
        None => interactive_init(&current_dir)?,
    };
    let (path, name, framework) = resolve_init(init, &current_dir)?;
    create_package(&path, &name, framework).map_err(|error| {
        format!(
            "Could not initialize project at {}: {error}",
            path.display()
        )
    })
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_path_and_name_defaults_and_override() {
        let cwd = Path::new("/tmp/current-project");
        for (args, path, name) in [
            (vec!["init"], cwd.to_path_buf(), "current-project"),
            (vec!["init", "."], cwd.to_path_buf(), "current-project"),
            (
                vec!["init", "next-project"],
                PathBuf::from("next-project"),
                "next-project",
            ),
            (
                vec!["init", "next-project", "--name", "custom"],
                PathBuf::from("next-project"),
                "custom",
            ),
        ] {
            let cli = Cli::try_parse_from(std::iter::once("necrass").chain(args)).unwrap();
            let Some(Commands::Init(init)) = cli.command else {
                panic!("expected init")
            };
            assert_eq!(
                resolve_init(init, cwd).unwrap(),
                (path, name.to_owned(), Framework::Axum)
            );
        }
    }

    #[test]
    fn rejects_invalid_invocations() {
        for args in [
            vec!["unknown"],
            vec!["init", "--name"],
            vec!["init", "first", "second"],
            vec!["init", "--name", "one", "--name", "two"],
            vec!["init", "--name", "bad name"],
            vec!["init", "--framework", "unknown"],
            vec!["init", "--unknown"],
        ] {
            assert!(Cli::try_parse_from(std::iter::once("necrass").chain(args)).is_err());
        }
    }

    #[test]
    fn validates_package_name_boundaries() {
        for name in ["a", "_", "API_2", "my-api", "my_api"] {
            assert_eq!(validate_name(name).unwrap(), name);
        }
        for name in ["", "2-api", "-api", "bad.name", "bad name", "한글", "api\n"] {
            assert!(validate_name(name).is_err(), "accepted {name:?}");
        }
    }

    #[test]
    fn requires_a_valid_derived_name_unless_overridden() {
        let cwd = Path::new("/tmp/current-project");
        for path in [Path::new("/"), Path::new("bad name")] {
            let init = || InitArgs {
                path: Some(path.to_owned()),
                name: None,
                framework: Framework::Actix,
            };
            assert!(resolve_init(init(), cwd).is_err());
            assert_eq!(
                resolve_init(
                    InitArgs {
                        name: Some("valid-api".into()),
                        ..init()
                    },
                    cwd
                )
                .unwrap(),
                (path.to_owned(), "valid-api".into(), Framework::Actix)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_directory_requires_an_explicit_package_name() {
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"api-\xff".to_vec()));
        let init = |name| InitArgs {
            path: Some(path.clone()),
            name,
            framework: Framework::Axum,
        };
        assert!(resolve_init(init(None), Path::new(".")).is_err());
        assert_eq!(
            resolve_init(init(Some("my-api".into())), Path::new(".")).unwrap(),
            (path, "my-api".into(), Framework::Axum)
        );
    }

    #[test]
    fn runs_explicit_initialization_for_each_framework_and_preserves_existing_files() {
        use std::ffi::OsStr;
        let directory = TestDirectory::new();
        for framework in ["axum", "actix"] {
            let project = directory.0.join(framework);
            let cli = || {
                Cli::try_parse_from([
                    OsStr::new("necrass"),
                    OsStr::new("init"),
                    project.as_os_str(),
                    OsStr::new("--name"),
                    OsStr::new("custom-api"),
                    OsStr::new("--framework"),
                    OsStr::new(framework),
                ])
                .unwrap()
            };
            run(cli()).unwrap();
            let manifest = fs::read_to_string(project.join("Cargo.toml")).unwrap();
            assert!(manifest.contains("name = \"custom-api\""));
            assert!(manifest.contains(&format!("necrassrs-{framework}")));
            let other = if framework == "axum" { "actix" } else { "axum" };
            assert!(!manifest.contains(&format!("necrassrs-{other}")));
            let main = fs::read_to_string(project.join("src/main.rs")).unwrap();
            let import = if framework == "axum" {
                "axum::"
            } else {
                "actix_web::"
            };
            assert!(main.contains(import));
            fs::write(project.join("src/resolvers.rs"), "user implementation").unwrap();
            let error = run(cli()).unwrap_err();
            assert!(error.contains("Could not initialize project at"), "{error}");
            assert!(
                error.contains("target is not an empty directory"),
                "{error}"
            );
            assert_eq!(
                fs::read_to_string(project.join("Cargo.toml")).unwrap(),
                manifest
            );
            assert_eq!(
                fs::read_to_string(project.join("src/resolvers.rs")).unwrap(),
                "user implementation"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn interactive_initialization_handles_defaults_selection_validation_and_cancellation() {
        use std::os::unix::process::CommandExt;
        const CHILD: &str = "NECRASSRS_CLI_WIZARD_TEST";
        if let Ok(expected_error) = std::env::var(CHILD) {
            let result = run(Cli { command: None });
            if expected_error.is_empty() {
                result.unwrap();
            } else {
                assert_eq!(result.unwrap_err(), expected_error);
            }
            return;
        }
        for (input, path, name, framework) in [
            (&b"\r\r\r"[..], "my-api", Some("my-api"), "axum"),
            (&b".\r\r\x1b[B\r"[..], ".", None, "actix"),
            (
                &b"nested/api\rwrong name\rvalid-api\r\x1b[B\r"[..],
                "nested/api",
                Some("valid-api"),
                "actix",
            ),
            (&b"\r\r\x1b"[..], "", None, ""),
        ] {
            let directory = TestDirectory::new();
            let (mut keyboard, terminal) = test_terminal();
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "tests::interactive_initialization_handles_defaults_selection_validation_and_cancellation"])
                .env(CHILD, if path.is_empty() { "Initialization cancelled." } else { "" })
                .env("TERM", "xterm")
                .current_dir(&directory.0)
                .stdin(terminal.try_clone().unwrap())
                .stderr(terminal)
                .stdout(std::process::Stdio::piped());
            // SAFETY: only async-signal-safe system calls run between fork and exec.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let mut child = command.spawn().unwrap();
            keyboard.write_all(input).unwrap();
            let started = std::time::Instant::now();
            let mut transcript = Vec::new();
            while child.try_wait().unwrap().is_none() {
                let mut buffer = [0; 1024];
                if let Ok(count) = std::io::Read::read(&mut keyboard, &mut buffer) {
                    transcript.extend_from_slice(&buffer[..count]);
                }
                if started.elapsed() > std::time::Duration::from_secs(10) {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    let output = child.wait_with_output().unwrap();
                    panic!(
                        "interactive initialization timed out for {input:?}: {}; {}",
                        String::from_utf8_lossy(&transcript),
                        String::from_utf8_lossy(&output.stdout)
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            if path.is_empty() {
                assert!(fs::read_dir(&directory.0).unwrap().next().is_none());
                continue;
            }
            let manifest = fs::read_to_string(directory.0.join(path).join("Cargo.toml")).unwrap();
            let name = name.unwrap_or_else(|| directory.0.file_name().unwrap().to_str().unwrap());
            assert!(manifest.contains(&format!("name = \"{name}\"")));
            assert!(manifest.contains(&format!("necrassrs-{framework}")));
            assert!(directory.0.join(path).join("src/resolvers.rs").is_file());
        }
    }

    #[cfg(unix)]
    fn test_terminal() -> (fs::File, fs::File) {
        use std::os::fd::{AsRawFd, FromRawFd};
        let (mut master, mut slave) = (-1, -1);
        let mut size = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: openpty receives valid output pointers and an initialized window size.
        let result = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        assert_eq!(result, 0, "{}", io::Error::last_os_error());
        // SAFETY: successful openpty transfers ownership of two distinct file descriptors.
        let (master, slave) =
            unsafe { (fs::File::from_raw_fd(master), fs::File::from_raw_fd(slave)) };
        let mut settings = std::mem::MaybeUninit::uninit();
        // SAFETY: tcgetattr initializes settings before it is read or passed to tcsetattr.
        unsafe {
            assert_eq!(libc::tcgetattr(slave.as_raw_fd(), settings.as_mut_ptr()), 0);
            let mut settings = settings.assume_init();
            libc::cfmakeraw(&mut settings);
            assert_eq!(
                libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &settings),
                0
            );
            assert_eq!(
                libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK),
                0
            );
        }
        (master, slave)
    }

    #[test]
    fn prepares_only_missing_or_empty_targets() {
        let directory = TestDirectory::new();
        let empty = directory.0.join("empty");
        prepare_target(&empty).unwrap();
        prepare_target(&empty).unwrap();

        let file = directory.0.join("file");
        fs::write(&file, "keep this").unwrap();
        let nonempty = directory.0.join("nonempty");
        fs::create_dir(&nonempty).unwrap();
        fs::write(nonempty.join("keep.txt"), "keep this").unwrap();
        for target in [&file, &nonempty] {
            assert_eq!(
                prepare_target(target).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
        }
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep this");
        assert_eq!(
            fs::read_to_string(nonempty.join("keep.txt")).unwrap(),
            "keep this"
        );

        let invalid_parent = file.join("child");
        assert!(prepare_target(&invalid_parent).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep this");

        #[cfg(unix)]
        for (index, destination) in [empty, directory.0.join("missing")].iter().enumerate() {
            let link = directory.0.join(format!("link-{index}"));
            std::os::unix::fs::symlink(destination, &link).unwrap();
            assert_eq!(
                prepare_target(&link).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(fs::read_link(&link).unwrap(), destination.as_path());
        }
    }

    #[test]
    fn creating_inside_a_workspace_does_not_edit_its_manifest() {
        let directory = TestDirectory::new();
        let parent_manifest = directory.0.join("Cargo.toml");
        let original = "[workspace]\nmembers = []\n";
        fs::write(&parent_manifest, original).unwrap();

        let child = directory.0.join("child");
        create_package(&child, "child", Framework::Axum).unwrap();

        assert_eq!(fs::read_to_string(parent_manifest).unwrap(), original);
        assert!(
            fs::read_to_string(child.join("Cargo.toml"))
                .unwrap()
                .contains("[workspace]")
        );
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "necrassrs-cli-unit-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
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
}

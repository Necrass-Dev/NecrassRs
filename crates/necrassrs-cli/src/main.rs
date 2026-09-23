use std::{fs, io, path::Path, process::Command};

fn create_package(path: &Path, name: &str) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_dir() && fs::read_dir(path)?.next().is_none() => {}
        Ok(_) => return Err(io::Error::from(io::ErrorKind::AlreadyExists)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(path)?,
        Err(error) => return Err(error),
    }

    let status = Command::new("cargo")
        .args(["init", "--bin", "--vcs", "none", "--name", name])
        .arg(path)
        .status()?;
    if !status.success() {
        return Err(io::Error::other("cargo init failed"));
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [command, path, option, name] = args.as_slice() else {
        eprintln!(
            "Usage: necrass init [PATH] [--name
        NAME]"
        );
        return std::process::ExitCode::FAILURE;
    };
    if command != "init" || option != "--name" {
        eprintln!(
            "Usage: necrass init [PATH] [--name
        NAME]"
        );
        return std::process::ExitCode::FAILURE;
    }
    let Some(name) = name.to_str() else {
        eprintln!("Project name must be valid UTF-8.");
        return std::process::ExitCode::FAILURE;
    };

    match create_package(Path::new(path), name) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "Could not initialize project:
            {error}"
            );
            std::process::ExitCode::FAILURE
        }
    }
}

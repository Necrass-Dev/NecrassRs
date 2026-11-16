---
title: CLI
description: Commands, arguments, target rules, and generated files of the necrass project initializer.
---

The `necrass` command creates new NecrassRs projects. It is installed separately
from the crates that applications depend on; see
[Install the CLI](/docs/installation/#install-the-cli). After initialization,
builds and schema changes use Cargo and [`necrassrs-build`](/docs/build-configuration/);
the CLI is not needed again.

This page describes the current source checkout. Use `necrass --help` and
`necrass init --help` for the usage of an installed version.

## Synopsis

```sh
necrass
necrass init [PATH] [--name <NAME>] [--framework <FRAMEWORK>]
necrass --help
necrass --version
```

## `necrass`

Without a subcommand, `necrass` asks for the project settings interactively:

| Prompt            | Default                                      |
| ----------------- | -------------------------------------------- |
| Project directory | `my-api`                                     |
| Package name      | The directory name, when it is valid Unicode |
| HTTP framework    | `axum`                                       |

An invalid package name is rejected at the prompt. Cancelling the framework
selection exits with `Initialization cancelled.` and creates no files.

Interactive mode requires both standard input and standard error to be terminals.
Otherwise, `necrass` prints help and exits with an error; use `necrass init` in
scripts.

## `necrass init`

Creates a project without asking questions.

| Argument                  | Default               | Description                                       |
| ------------------------- | --------------------- | ------------------------------------------------- |
| `PATH`                    | The current directory | Target directory. `.` also means the current one. |
| `--name <NAME>`           | The target's name     | Cargo package name.                               |
| `--framework <FRAMEWORK>` | `axum`                | HTTP framework: `axum` or `actix`.                |

A package name must start with an ASCII letter or underscore and contain only
ASCII letters, digits, hyphens, or underscores. When the directory name is not a
valid package name, or cannot be read as Unicode, pass `--name` explicitly.

See [Supported frameworks](/docs/integration/#supported-frameworks) for the
adapter each `--framework` value selects.

## Target directory

| Target                                     | Result                       |
| ------------------------------------------ | ---------------------------- |
| Does not exist                             | Created, including parents   |
| Empty directory                            | Used                         |
| Nonempty directory, file, or symbolic link | Rejected; nothing is written |

Each file is created only if it does not already exist, so initialization never
overwrites existing files. The generated manifest declares its own `[workspace]`,
and a parent Cargo workspace manifest is not edited.

## Generated files

| Path                    | Contents                                                                                     |
| ----------------------- | -------------------------------------------------------------------------------------------- |
| `Cargo.toml`            | Package, runtime and adapter dependencies, `necrassrs-build`, and `panic = "abort"` profiles |
| `README.md`             | Starter instructions                                                                         |
| `build.rs`              | Calls `necrassrs_build::build("schema")` and reports build errors                            |
| `schema/schema.graphql` | Starter SDL with `Query.hello`                                                               |
| `src/main.rs`           | HTTP server for the selected framework                                                       |
| `src/generated.rs`      | Includes the generated contracts from `OUT_DIR`                                              |
| `src/resolvers.rs`      | Resolver implementation for the starter schema                                               |

The dependencies are released NecrassRs packages from crates.io. See
[Build Configuration](/docs/build-configuration/) for what `build.rs` generates
and synchronizes.

## Exit status

`necrass` exits with status 0 on success, including `--help` and `--version`.
Invalid arguments, such as an unknown framework or an invalid `--name`, are
reported by the argument parser with status 2. Other failures, including a
rejected target, print the error to standard error and exit with status 1.

---
title: Installation
description: Define a GraphQL API in SDL and build its Rust contracts with NecrassRs.
---

## Install the CLI

Install Rust and Cargo using the [official Rust installation guide](https://rust-lang.org/tools/install/).
Use a current stable toolchain. Then install the CLI from crates.io:

```sh
cargo install necrassrs-cli --locked
necrass --version
```

The Cargo package is named `necrassrs-cli`; the installed executable is named
`necrass`. If your shell cannot find it, ensure Cargo's binary directory is on
`PATH` (`~/.cargo/bin` on Unix or `%USERPROFILE%\.cargo\bin` on Windows).

### Prebuilt binaries

You can also download the CLI from [GitHub Releases](https://github.com/Necrass-Dev/NecrassRs/releases).
Choose the archive for your operating system and processor:

| Platform                | Rust target                | Archive                                             |
| ----------------------- | -------------------------- | --------------------------------------------------- |
| Linux x86_64 (GNU libc) | `x86_64-unknown-linux-gnu` | `necrass-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| macOS Intel             | `x86_64-apple-darwin`      | `necrass-<version>-x86_64-apple-darwin.tar.gz`      |
| macOS Apple Silicon     | `aarch64-apple-darwin`     | `necrass-<version>-aarch64-apple-darwin.tar.gz`     |
| Windows x86_64          | `x86_64-pc-windows-msvc`   | `necrass-<version>-x86_64-pc-windows-msvc.zip`      |

These binaries are built and checked on Ubuntu 24.04, macOS 15, and Windows
Server 2025 runners, respectively. Older systems and other architectures are
not checked by the release workflow; use Cargo to build for your system instead.
Rust and Cargo are still required to build the projects created by the CLI.

Before extracting the archive, compare its SHA-256 with the digest displayed
beside that file on the GitHub Release page. GitHub generates this digest when
the file is uploaded; there is no separate checksum file. You can also retrieve
the `digest` field through the [release assets API](https://docs.github.com/en/rest/releases/assets).

Use the downloaded filename in the appropriate command:

```sh
# Linux
sha256sum "<archive>.tar.gz"

# macOS
shasum -a 256 "<archive>.tar.gz"
```

```powershell
# Windows PowerShell
Get-FileHash "<archive>.zip" -Algorithm SHA256
```

The hexadecimal hash must match GitHub's value (ignoring letter case and any
`sha256:` prefix). If it differs, do not use the archive.

Extract the `.tar.gz` with `tar -xzf "<archive>.tar.gz"`, or the `.zip` with
PowerShell's `Expand-Archive "<archive>.zip" -DestinationPath necrass-cli`.
Each archive contains `necrass` (or `necrass.exe`) and `LICENSE.md`. Put the
executable in a directory on `PATH`, then run `necrass --version`.

## Create a project

Run the CLI without arguments in a terminal:

```sh
necrass
```

It asks for a project directory, Cargo package name, and backend framework. The
default directory is `my-api`, and the package name defaults to the directory name.
Choose a supported backend framework from the available options.

For scripts or when you already know the settings, use an explicit command:

```sh
necrass init my-api
```

This creates a project without asking questions. Use `--name <NAME>` to override
the Cargo package name and `--framework <FRAMEWORK>` to select a backend framework.

Without a path, `necrass init` uses the current directory. Targets must be new or empty;
existing projects, nonempty directories, and symbolic-link targets are rejected.

Use `necrass --help` and `necrass init --help` for command usage. Without a terminal,
supply an explicit command; bare `necrass` prints help and exits with an error.

The starter uses released NecrassRs packages from crates.io. Cargo records the
resolved package versions in the new project's `Cargo.lock` on the first build.
Keep that lockfile for repeatable builds.

## Run the server and send a request

If you accepted the `my-api` directory or used the commands above:

```sh
cd my-api
cargo run
```

Both starters serve GraphQL on GET and POST `/graphql` and GraphiQL on GET
`/graphiql` at `http://127.0.0.1:3000`. Open
[GraphiQL](http://127.0.0.1:3000/graphiql) and run:

```graphql
{
  hello(name: "Sheri")
}
```

Or send the request from another terminal:

```sh
curl -sS http://127.0.0.1:3000/graphql \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/graphql-response+json' \
  --data '{"query":"{ hello(name: \"Sheri\") }"}'
```

The response is:

```json
{ "data": { "hello": "Hello, Sheri" } }
```

GraphiQL loads browser assets from `esm.sh` and needs network access. See
[Introspection and GraphiQL](/docs/graphiql/) for UI and introspection settings.

## From schema to implementation

1. Define your API in `schema/**/*.graphql`.
2. Run `cargo build` to generate contracts and synchronize `src/resolvers.rs`.
3. Implement new resolver bodies and run the server.

Retained fields keep their existing resolver bodies. New fields receive
`unimplemented!()` stubs, which you must implement before calling them. Deleting
or renaming a field removes an untouched generated stub. If you edited its body,
attributes, or comments, synchronization stops until you remove or migrate it.

After initialization, ordinary builds and runs use Cargo; the CLI is not needed
for schema changes. The starter configures `panic = "abort"` in development and
release builds, so calling an unimplemented resolver terminates the process.
Return `ResolverError` for ordinary application errors.

## Explore the documentation

- [Tutorial](/docs/tutorial/): build a web server and query it with GraphiQL or HTTP.
- [Personnel Management System](/docs/personnel-management/): add Object, Interface,
  and Union results to a working server.
- [Types](/docs/types/): Rust type mappings, input presence, defaults, OneOf, and recursion.
- [Custom Scalars](/docs/custom-scalars/): define scalar storage, coercion, and
  serialization.
- [Organizing Resolver Files](/docs/resolver-files/): split implementations across
  Rust modules and understand synchronization safety.
- [Introspection and GraphiQL](/docs/graphiql/): development UI and introspection settings.

These pages follow the source checkout from which this site is built; they do not
describe a versioned release. Use [docs.rs](https://docs.rs/necrassrs/) for the
API reference of a published version.

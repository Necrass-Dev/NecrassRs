# NecrassRs architecture and development plan

Date: 2026-09-17
Updated: 2026-09-19

This document defines the target product structure, crate responsibilities, development and release practices, and consumer workflow. It does not describe a completed implementation. Package names and directory layouts are proposed; concrete Rust API signatures remain subject to design.

## 1. Product purpose

NecrassRs is a server framework that treats GraphQL SDL as the public API contract and generates Rust types, resolver contracts, object wrappers, and execution wiring. Applications implement state and business logic in separate Rust code.

The product has three entry points:

| Entry point | When used | Responsibility |
| --- | --- | --- |
| `necrass init` | Project initialization | Prepare SDL, Cargo configuration, user implementations, and a server entry point |
| `necrassrs-build` | Consumer builds | Validate SDL and generate Rust code from `build.rs` |
| `necrassrs` and an HTTP adapter | Server execution | Validate requests, invoke user resolvers, and construct responses |

After initialization, building and running the application does not require an installed CLI. The build script calls a Rust library, not a CLI subprocess. Separate `necrass compile` and watch commands are out of scope.

## 2. Design direction

- SDL is the sole source of the public GraphQL contract. Do not synchronize Rust implementations back into SDL.
- Use Apollo Compiler's schema models and validation rather than duplicating a GraphQL schema model and validator.
- Parsing uses `apollo-parser` through Apollo Compiler. Do not add a direct parser dependency unless direct CST access is needed.
- Adopting Apollo's models and validation does not select its execution engine. Execution remains an open decision.
- Use four packages in one Cargo workspace: the runtime, build library, Axum adapter, and CLI. Keep code generation and Cargo integration as separate modules within the build library.
- Axum is the first officially supported HTTP adapter. The execution core does not depend on Axum.
- Applications own their Context types and construction.
- Generated code belongs in `OUT_DIR`; user implementations belong in `src`. Regeneration must not edit user files.
- Treat every output field as a resolver. Argument presence does not determine whether a field is an automatic getter.
- Allow partially implemented applications to build and run. Calling an unimplemented field panics, with process termination enforced by the executable's policy.
- Do not spawn independent tasks for individual fields by default. Target a `Send` execution future with request-scoped borrowing.
- Do not reuse prototype code. Carry forward observations and validation scenarios as acceptance criteria.

## 3. Apollo Compiler usage

### 3.1 Responsibilities supplied by the dependency

At build time, use SDL parsing, schema validation, and the `Schema` model. At runtime, parse and validate request documents against the schema and use `ExecutableDocument`.

| Area | Apollo Compiler contribution | NecrassRs responsibility |
| --- | --- | --- |
| SDL | Parsing, semantic validation, type and field lookup | Supported-feature restrictions and Rust generation checks |
| Code generation | Validated schema model | Rust naming and type mapping, traits, wrappers, and dispatch |
| Request documents | Parsing and schema-aware document validation | Execution entry points and error-response integration |
| Input processing | Coercion APIs remain under review | Assign responsibility for runtime variable values and custom scalar conversion |
| Execution | Execution APIs are evaluated separately | Context and resolver integration and the complete execution contract |

Static request validation does not establish that runtime variable coercion or custom scalar input validation has completed. Define these boundaries in the execution design.

### 3.2 No separate schema crate for now

The current architecture does not include `necrassrs-schema`. Do not copy Apollo's entire type, field, and argument model into local types or create a crate solely to wrap dependencies.

Keep generation-specific information, such as Rust identifiers, generated type names, and recursive input representations, inside the codegen module of `necrassrs-build`. Extract a shared model or policy only when a concrete NecrassRs requirement spans the generator and runtime.

Distinguish consumer APIs from internal schema representation. Users should not need Apollo internals to implement resolvers or Context. Generated code should access runtime contracts through public `necrassrs` paths.

### 3.3 Adoption limits

Apollo Compiler targets specification compliance; this does not establish exhaustive correctness for every feature in a particular version. Select the target GraphQL specification edition and NecrassRs support scope separately.

Upstream release notes include fixes for interface implementation types and fragment validation. Record dependency versions and retain regression checks for important integration paths and known failures. Do not promise stable diagnostic wording or drive behavior by parsing error strings.

The prototype's Apollo Compiler 1.32.0 execution path exhibited incorrect nullable-list-item error propagation and a non-`Send` execution future. These must be addressed when choosing an executor; they do not automatically disqualify the schema models and validation.

## 4. Crates

| Package | Responsibility | Direct consumers |
| --- | --- | --- |
| `necrassrs` | Public runtime API and integration of request validation, input processing, resolver execution, and response completion | Applications and HTTP adapters |
| `necrassrs-build` | Discover and validate SDL, generate Rust types, traits, wrappers, and dispatch, emit Cargo rebuild instructions, and manage output | Consumer `build.rs` |
| `necrassrs-axum` | GraphQL HTTP extraction, response conversion, and development UI integration | Axum applications |
| `necrassrs-cli` | The `necrass` binary, initialization templates, and file creation | Developers |

Within `necrassrs-build`, keep code generation independent of Cargo environment variables and filesystem operations so it can be tested directly. Cargo integration manages inputs, rebuild instructions, and output. The runtime does not depend on the build library.

There is no separate codegen package: it currently has no independent consumer or release lifecycle. A module boundary preserves testability without adding another published dependency. Extract a crate only when another tool needs independent reuse or a concrete dependency or release boundary emerges.

An initial internal layout is sufficient:

```text
necrassrs-build/src/
├── lib.rs       # Public build API and Cargo integration
└── codegen.rs   # Validated schema to Rust code
```

The CLI prepares projects from templates. If initialization needs schema processing or generation, reuse the corresponding library instead of duplicating its logic.

Do not create `necrassrs-http` yet. Consider extracting common HTTP behavior when multiple official adapters need it. Do not add a facade that only re-exports the runtime or a generic executor-backend trait without a concrete requirement.

## 5. Dependencies and data flow

Arrows below indicate dependencies.

```text
Consumer build.rs
  └─ necrassrs-build
       └─ apollo-compiler

Consumer server and generated code
  ├─ necrassrs
  │    └─ apollo-compiler
  └─ necrassrs-axum
       ├─ necrassrs
       └─ axum

necrassrs-cli
  └─ Project initialization templates
```

A build-time `Schema` instance does not survive into the running server. How generated applications obtain their runtime schema remains open. Compare embedding SDL and parsing during initialization with generating schema-construction code; do not invent a schema serialization format first.

The complete workflow is:

```text
cargo install necrassrs-cli --locked
  → Install the necrass executable
  → necrass init in an empty project directory
  → Create Cargo.toml, build.rs, SDL, and user source files

Edit SDL
  → cargo build
  → build.rs
  → Apollo schema parsing and validation
  → NecrassRs support and Rust naming checks
  → Generate Rust contracts and wiring in OUT_DIR
  → Compile generated code with user implementations

cargo run
  → User-owned Axum server
  → Extract GraphQL request and construct Context in the handler
  → Validate the request document and process inputs
  → Execute selected resolvers
  → Complete the response, including errors and null propagation
  → HTTP response
```

The diagrams in this document were drafted with Codex to explain the proposed architecture. They are not execution or measurement evidence and remain subject to human review.

## 6. HTTP and Context

The execution core accepts GraphQL request information and user Context, not an HTTP request object. Exact signatures remain open, but the API must support query, variables, operationName, and Context.

`necrassrs-axum` provides extraction and response conversion that users can compose in their handlers. It does not require a dedicated server runner, authentication middleware, or Context factory callback.

| Application responsibility | NecrassRs responsibility |
| --- | --- |
| Routing, middleware, and server lifecycle | GraphQL execution entry point |
| Authentication and header/State extraction | Passing Context to resolvers |
| Context type and per-request construction | Keeping request contexts isolated |
| Database, ORM, and service selection | Connecting SDL contracts to user implementations |
| Operational configuration and deployment | Documented type, execution, and error contracts |

Context may contain authenticated identity and shared resources such as database pools, or use the unit type when no data is needed. Applications choose how shared state and request-specific data are combined.

Resolver futures must be able to borrow `self` and Context for the call lifetime rather than requiring every call to be `'static`. Target a `Send` future for the entire execution path. Define the required `Send` and `Sync` bounds on Context and shared types in the public API. Do not require futures themselves to be `Sync` by default.

Avoiding per-field spawning does not mean serializing all fields. Define within-request concurrency and mutation ordering separately. Dropping a request future should not leave detached framework field tasks behind. Adapters determine how network disconnection cancels execution; cancellation does not roll back external effects that have already occurred.

## 7. Initialization and consumer projects

### 7.1 MVP initialization

The MVP `necrass init` creates a small, runnable Axum application in an empty project directory, including `Cargo.toml`, `build.rs`, SDL, and user source files. Users do not need to add dependencies before initialization. Do not add framework selection options or empty templates for other adapters.

The intended installation and initialization flow is shown below. These commands describe the planned product, not an available release:

```sh
cargo install necrassrs-cli --locked
mkdir my-api
cd my-api
necrass init
```

The package is named `necrassrs-cli`; its installed executable is named `necrass`. It is a separately installed development tool, not a consumer `dev-dependency`. Adding a package to `[dev-dependencies]` does not install its executable as a shell command.

The MVP does not merge dependencies into an existing `Cargo.toml`. Existing applications follow manual integration instructions. Exact CLI arguments remain open; automatic existing-project integration is deferred until its configuration-preservation behavior is designed.

Generated server code is ordinary application-owned source. The execution core remains independent of Axum. Existing projects can integrate `necrassrs`, `necrassrs-build`, and `necrassrs-axum` without using the CLI.

```text
my-api/
├── Cargo.toml
├── build.rs
├── schema/
│   └── schema.graphql
└── src/
    ├── main.rs
    ├── context.rs
    ├── generated.rs
    └── resolvers.rs
```

| File | Ownership and purpose |
| --- | --- |
| `schema/*.graphql` | User-written public GraphQL contract |
| `build.rs` | Build-library invocation and input configuration |
| `main.rs` | User routing, handlers, and server configuration |
| `context.rs` | User Context definition |
| `resolvers.rs` | User state and business logic |
| `generated.rs` | Module that includes generated code from `OUT_DIR` |
| Rust files in `OUT_DIR` | Automatically generated; not edited manually |

Initialization writes the appropriate dependency sections using a tested release combination:

| Installation or manifest location | Contents |
| --- | --- |
| `cargo install` | `necrassrs-cli`, providing the `necrass` executable |
| `[dependencies]` | `necrassrs`, `necrassrs-axum`, Axum, the async runtime, and other libraries directly used by the application |
| `[build-dependencies]` | `necrassrs-build`, called by `build.rs` |
| `[dev-dependencies]` | Test and example dependencies only when needed; not the CLI |

Ordinary consumers should not need a direct Apollo Compiler dependency. Once initialized, the application builds and runs with Cargo without an installed CLI.

### 7.2 Everyday development

1. Install the CLI and initialize an empty project directory, or manually add the libraries and build script to an existing application.
2. Define objects, fields, arguments, and nullability in SDL.
3. Generate Rust contracts and wiring through Cargo.
4. Implement generated resolver traits on user structs.
5. Return user objects through generated wrappers from parent resolvers.
6. Construct Context in the handler and pass it to execution.
7. Run the server and test completed fields.
8. Repeat as SDL and business logic evolve.

Constructing a wrapper does not execute child fields. Execution invokes only selected fields. Cargo regeneration must track SDL changes, additions, and deletions while preserving user source files.

### 7.3 Partial implementations and contract changes

The recommended approach is to generate trait default methods using `unimplemented!()`. Users override the methods they need, and adding fields does not automatically edit their source. Final adoption and how initial example implementations are presented remain open.

Default methods prevent the compiler from detecting omitted implementations. Existing trait implementations that no longer match deleted, renamed, or changed SDL fields still produce Rust compilation errors.

Requests that do not select unimplemented fields must remain executable. Calling an unimplemented field must panic and terminate the process rather than recover as a GraphQL field error. Ordinary domain errors follow a separate GraphQL error path.

Enforce termination through executable configuration such as `panic = "abort"`. Library profiles do not propagate to consumers. Initialization templates and manual setup documentation must explain configuration at the application or workspace root. This policy affects all panics, not only unimplemented fields.

## 8. Repository structure

```text
necrassrs/
├── Cargo.toml
├── Cargo.lock
├── crates/
│   ├── necrassrs/
│   ├── necrassrs-build/
│   ├── necrassrs-axum/
│   └── necrassrs-cli/
│       └── templates/
├── examples/
│   └── basic-server/
├── tests/
│   └── integration/
│       └── Cargo.toml
├── docs/
│   └── architecture.md
└── .github/
    └── workflows/
```

Use a virtual workspace at the root. Keep unit tests with their crates and cross-package checks in a separate integration package. Maintain the example server as a real consumer package with a build script, Context, resolvers, and HTTP integration.

Manage edition, MSRV, license, shared dependency declarations, and lint policy at the workspace root. Select the MSRV alongside dependency requirements. Keep build-tool and CLI dependencies out of the runtime.

Official repository documentation is written in English. Agent workflow and AI-assisted contribution requirements are defined in [AGENTS.md](../AGENTS.md) and [AI_POLICY.md](../AI_POLICY.md).

## 9. Dependencies, versions, and releases

- Commit one root `Cargo.lock` to reproduce validated dependency combinations in development and CI.
- Periodically test updated dependency resolutions in addition to locked builds.
- Consumers resolve dependencies with their own lockfiles. The repository lockfile does not pin consumer dependencies.
- Use `path + version` for local package dependencies. Publish internal crates required as build or runtime dependencies of public packages.
- Mark examples and integration packages `publish = false`.
- Initially version and release all four product packages together, including unchanged packages when necessary. This avoids independent release schedules and compatibility matrices at this stage. Release automation remains a separate decision.
- Define compatibility between `necrassrs-build` and `necrassrs`, including runtime contracts called by generated code. Publishing matching versions alone does not prevent incompatible consumer combinations. Validate the supported combination with the consumer example and use that combination in CLI templates.
- Before release, verify packaging and consumer builds without relying on local-only paths.

Apollo Compiler documents testing on the latest stable Rust. Check the NecrassRs MSRV separately when updating dependencies.

## 10. Validation and acceptance criteria

| Area | Main criteria |
| --- | --- |
| Schema and generation | Reject invalid SDL, diagnose unsupported features, detect Rust naming collisions |
| Cargo integration | Track SDL changes/additions/deletions and preserve user files |
| Public contracts | Compile consumer implementations, diagnose contract changes, allow request borrowing, ensure the entire execution future is `Send` |
| Partial implementation | Do not call unselected fields; execute implemented fields; terminate separate dev and release executables on unimplemented calls |
| GraphQL execution | Preserve input states, variables, defaults, coercion, selection rules, error paths, null propagation, and mutation order |
| Known regression | Nullable list-item conversion errors stop at the correct nullable boundary |
| HTTP | Convert requests/responses, limit bodies, isolate Context, distinguish request and execution errors |
| Initialization and release | Build/run initialized projects, refuse file/symlink overwrites, support packaged dependencies |

Test process termination with separate executable processes, not by catching a panic inside the test runner. When providing GraphiQL, define development introspection settings and UI asset delivery as well.

This table is not a claim of exhaustive GraphQL conformance. Specify detailed feature support and conformance coverage once the specification edition and execution engine are selected.

## 11. Open decisions

1. **Execution engine:** Whether to adopt Apollo execution, how to address list-error and `Send` issues, and the responsibilities of alternatives.
2. **Public resolver contract:** Context type integration, arguments, errors, wrapper ownership and lifetimes, and internal type erasure.
3. **Build/runtime boundary:** Runtime schema construction and the generated dispatch contract.
4. **Partial implementation:** Adoption of trait default methods and initial user implementation scaffolding.
5. **GraphQL scope:** Specification edition, supported types/features, explicitly rejected features, runtime variable and custom scalar validation boundaries.
6. **HTTP and development UI:** Methods, media types, status codes, introspection settings, and asset distribution.
7. **Release contract:** MSRV, default features, generator/runtime compatibility, and CLI initialization details.

SQL generation, ORM integration, automatic batching, a separate non-`Send` mode, standalone watch, and performance optimization are not prerequisites for this architecture.

## 12. References

- Internal design basis: the 2026-09-17 prototype report, cumulative development article, and subsequent architecture discussions. Do not reproduce private development records in the public repository.
- [Apollo Compiler API](https://docs.rs/apollo-compiler/1.32.0/apollo_compiler/): models, parsing, and validation.
- [Apollo Compiler changelog](https://github.com/apollographql/apollo-rs/blob/main/crates/apollo-compiler/CHANGELOG.md): versioned features and specification-related fixes.
- [Apollo project and Rust version policy](https://github.com/apollographql/apollo-rs): purpose, license, and support policy.
- [Validation error codes, issue #855](https://github.com/apollographql/apollo-rs/issues/855): diagnostic wording versus programmatic error contracts.
- [Cargo lockfile guidance change](https://blog.rust-lang.org/2023/08/29/committing-lockfiles/): background on committing library lockfiles.
- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html): shared workspace configuration.
- [Cargo dependency locations](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#multiple-locations): local paths, versions, and registry publication.

# NecrassRs architecture and development plan

Date: 2026-09-17
Updated: 2026-09-22

This document defines the target product structure, crate responsibilities, development and release practices, and consumer workflow. It does not describe a completed implementation. Package names and directory layouts are proposed; concrete Rust API signatures remain subject to design.

## 1. Product purpose

NecrassRs is a server framework that treats GraphQL SDL as the public API contract. Cargo builds generate Rust contracts and execution wiring and create editable resolver implementations from SDL. Applications fill the generated method bodies with business logic. Later builds synchronize those implementation declarations with SDL instead of requiring users to maintain the scaffolding by hand.

The product has three entry points:

| Entry point | When used | Responsibility |
| --- | --- | --- |
| `necrass init` | Project initialization | Create `build.rs`; full project scaffolding is a later convenience |
| `necrassrs-build` | Consumer builds | Validate SDL, generate contracts in `OUT_DIR`, and create/synchronize editable resolver implementations in `src` |
| `necrassrs` and an HTTP adapter | Server execution | Validate requests, invoke user resolvers, and construct responses |

After initialization, building and running the application does not require an installed CLI. The build script calls a Rust library, not a CLI subprocess. Separate `necrass compile` and watch commands are out of scope.

## 2. Design direction

- SDL is the sole source of the public GraphQL contract. Do not synchronize Rust implementations back into SDL.
- Use Apollo Compiler's schema models and validation rather than duplicating a GraphQL schema model and validator.
- Parsing uses `apollo-parser` through Apollo Compiler. Do not add a direct parser dependency unless direct CST access is needed.
- Implement execution inside `necrassrs`, retaining Apollo's models and validation. Do not require an Apollo executor patch or a separate executor crate.
- Use four packages in one Cargo workspace: the runtime, build library, Axum adapter, and CLI. Keep code generation and Cargo integration as separate modules within the build library.
- Axum is the first officially supported HTTP adapter. The execution core does not depend on Axum.
- Applications own one Context type per schema and construct its values per request. Generate a struct for each field's arguments.
- Disposable contracts and dispatch code belong in `OUT_DIR`. Editable resolver structs and explicit method implementations belong in `src/resolvers.rs`. Build-time synchronization updates SDL-owned declarations there, preserves bodies of retained methods, adds `unimplemented!()` stubs, and removes methods whose SDL fields were deleted. Unrelated user code must not be overwritten.
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
| Code generation and synchronization | Validated schema model | Rust naming and type mapping, traits, wrappers, dispatch, and AST-based reconciliation of editable resolver declarations |
| Request documents | Parsing, schema-aware document validation, and operation selection | Execution entry points and error-response integration |
| Input processing | Public variable-value coercion API | Field-argument processing, generated argument conversion, and custom scalar integration when supported |
| Execution | Validated schema and executable-document models | Field collection, resolver dispatch, Context propagation, result coercion, errors, and null propagation |

Static request validation does not establish that runtime variable coercion or custom scalar input validation has completed. Reuse `request::coerce_variable_values` for variables. Apollo Compiler 1.32.0's field-argument coercion implementation is crate-private; do not rely on it as a consumer API.

### 3.2 No separate schema crate for now

The current architecture does not include `necrassrs-schema`. Do not copy Apollo's entire type, field, and argument model into local types or create a crate solely to wrap dependencies.

Keep generation-specific information, such as Rust identifiers, generated type names, and recursive input representations, inside the codegen module of `necrassrs-build`. Extract a shared model or policy only when a concrete NecrassRs requirement spans the generator and runtime.

Distinguish consumer APIs from internal schema representation. Users should not need Apollo internals to implement resolvers or Context. Generated code should access runtime contracts through public `necrassrs` paths.

#### Generated argument names

Generated field argument structs live at `generated::types::<object>::<field>::Args` in the consumer crate. For example, `Query.hello(name: String!)` produces `generated::types::Query::hello::Args` with a public `name: String` field. Preserve SDL case and object/field boundaries rather than concatenating or case-converting names. `Query.hello` and `Query.Hello` therefore have distinct paths.

Apply the same injective identifier mapping to object modules, field modules, and argument fields:

- Prefix `self`, `Self`, `super`, and `crate` with one underscore.
- Prefix every SDL name already starting with an underscore with one additional underscore, including `_` itself. Thus `self` maps to `_self`, `_self` to `__self`, and `_` to `__`.
- Preserve all other names. Emit mapped names as Rust raw identifiers so keywords such as `type` and `gen` remain usable. The `r#` syntax does not change identifier identity.

For example, `Query.type(self: String!, _self: String!)` produces `types::Query::r#type::Args` with distinct `_self` and `__self` fields. Consumers can omit `r#` for non-keywords. Permit `non_snake_case` only within the generated `types` module, and qualify standard-library types to avoid name shadowing.

Each field module reserves its own `Args` type; SDL field names occupy the parent object module instead. Keep future generated helpers separate from SDL-derived namespaces. This mapping does not rename SDL fields or change runtime field coordinates. Verify the mapping by compiling generated consumer code, including case differences, underscore boundaries, keywords, and raw-identifier exceptions.

The injective mapping is also the identity rule for source synchronization. Compare the desired SDL-derived declarations with the existing Rust AST using mapped object and field names; do not match by position, similar spelling, or method body. Raw-identifier spelling does not change identity. A field rename is deletion of the old method and addition of a new stub, never migration of its old body.

Resolver traits live in `generated::resolvers`. Append the fixed suffix `Resolver` to the mapped object name without changing case: `User`, `user`, and `UserResolver` become `UserResolver`, `userResolver`, and `UserResolverResolver`. Allow `non_camel_case_types` and `non_snake_case` on these generated traits. Each trait has a generic Context parameter, and each field produces a method using the same identifier mapping.

Methods borrow `self` and Context for the call lifetime, take the field's generated `Args` by value, and return `impl Future<Output = Result<T, necrassrs::ResolverError>> + Send` with that lifetime. Fields without arguments use an empty `Args` struct. Default methods return a future that calls `unimplemented!()` when polled, allowing partial trait implementations to compile. The current generator supports `String!` argument and return types; other argument and return types produce generation errors.

Trait defaults are a low-level fallback, not the user-facing scaffolding workflow. The build integration must also create the concrete resolver struct and an explicit editable async method for every supported SDL field. Users replace the `unimplemented!()` body in that implementation; they do not have to copy trait signatures or write an empty trait implementation first. The generated implementation must satisfy the existing borrowing, Context, and `Send` contracts.

Generate one `generated::dispatch::SchemaDispatcher` for the schema, rather than separate dispatcher types for each root. The current query-only implementation stores the application-owned Query value via `SchemaDispatcher::new(query)` and implements `necrassrs::Dispatcher<C>` with `C: Sync` and `Q: QueryRootResolver<C> + Sync`, using the actual query root's generated trait. It matches original SDL type/field coordinates, converts prepared arguments to the generated `Args`, invokes the resolver, converts successful results to `JsonValue`, and preserves `ResolverError` values. Unknown coordinates and invalid prepared arguments return errors without panicking. Schemas with mutation or subscription roots currently produce a generation error; their routing remains unimplemented.

### 3.3 Adoption limits

Use the GraphQL September 2025 specification as the reference for supported behavior. The greeting MVP's scope and completion criteria are already defined in [issue #1](https://github.com/Necrass-Dev/NecrassRs/issues/1). Neither dependency adoption nor the MVP implies complete GraphQL conformance.

#### MVP type support and current implementation

Keep the generated API's type scope limited to `String!` arguments and results until issue #1 is complete. Establish expansion principles now; implement additional type support in follow-up issues rather than expanding the greeting MVP.

Apollo schema validation establishes GraphQL validity, not NecrassRs code generation or execution support. Treat a type as supported through the generated API only when Rust generation, input conversion, dispatch, and runtime result completion work together and are tested.

Current implementation status:

- The generator produces argument structs and resolver methods for `String!`, including empty argument structs and default methods for partial implementations. Consumer compilation checks cover naming, borrowed Context values, and `Send` resolver futures.
- The runtime completes String results, including nullable and list combinations, but does not generally complete other scalar, enum, or object results. Its broader input processing and Apollo validation do not establish complete type support.
- Generated query dispatch executes through the runtime with borrowed Context and a `Send` execution future. Executable consumer checks cover the greeting, custom root/field names, argument conversion failures, domain-error preservation, and successful execution without selecting an unimplemented field.
- The generator exposes `generated::SDL` as a public string constant using Apollo's schema serialization. Executable consumer checks reconstruct the runtime schema from it, including definitions and extensions from multiple sources.
- A Linux/macOS subprocess test builds a partial consumer with `panic = "abort"` in both Cargo dev and release profiles. It checks successful execution of an implemented field and `SIGABRT` termination without a response when an unimplemented field is selected.
- `CodegenError` implements `miette::Diagnostic`, retaining the original named source and available Apollo byte spans without reading files. Argument-type labels cover the complete type reference; return-type labels identify the named type. Missing locations or source entries leave the message available without fabricated sources or labels. Tests cover argument diagnostics across multiple sources, UTF-8 byte offsets, and type nodes without locations. The consumer build-script fixture renders codegen diagnostics with miette and preserves Apollo's own diagnostic rendering; the library does not enable `fancy`.
- The pure codegen tests verify what happens when explicitly supplied Rust implementations are compiled against changed contracts, including `E0407` and `E0609`. They do not exercise implementation-file synchronization and must not be interpreted as its intended workflow. In the target Cargo workflow, deleted or renamed fields are removed from the existing implementation AST. A retained user body may still fail to compile if it uses an argument that the new contract removed; synchronization does not rewrite business logic.
- The current `build()` implementation recursively reads SDL, validates a combined Apollo schema, writes disposable code to `OUT_DIR/necrassrs.rs`, and emits Cargo rebuild instructions. This is the first stage of build integration, not complete resolver scaffolding. Creating and synchronizing `src/resolvers.rs` is not yet implemented. The replacement Cargo acceptance tests start without that file and describe the required flow; their presence is not evidence that synchronization works.

The support contract requires explicit diagnostics for unsupported schema features, with source locations when available. Do not silently map unsupported types to String or treat Apollo validation as proof that generation will succeed. Keep validation diagnostics separate from generation errors, and do not make temporary limitations such as lack of Int support permanent rejection contracts.

Type expansion must preserve SDL identity and nullability, distinguish omitted nullable inputs from explicit null, and preserve list-container and list-item nullability independently. `Option<T>` for nullable outputs and `Vec<T>` for lists are design candidates; concrete public representations for ID, input presence, enums, objects, and custom scalars remain uncommitted until their implementation and consumer contracts are validated. Do not add speculative public types for these future features during the MVP.

Upstream release notes include fixes for interface implementation types and fragment validation. Record dependency versions and retain regression checks for important integration paths and known failures. Do not promise stable diagnostic wording or drive behavior by parsing error strings.

The prototype's Apollo Compiler 1.32.0 execution path exhibited incorrect nullable-list-item error propagation and a non-`Send` execution future. Its resolver-error boundary also lost application error codes and prefixed messages. The list behavior concerns specification correctness; `Send`, extension preservation, and exact MVP message wording are NecrassRs integration requirements. These findings do not disqualify Apollo's schema models and validation.

### 3.4 Feasibility evidence and limits

A disposable experiment using unmodified Apollo Compiler 1.32.0 executed the greeting example without calling Apollo's executor. It reused document validation, operation selection, public variable coercion, and response error types. The caller parsed the fixed schema once and borrowed it during execution.

| Verified in the experiment | Evidence |
| --- | --- |
| Complete execution future is `Send` | Compile-time bound on the future, including a resolver suspension point |
| Resolver and Context borrowing | Borrowed request-local values survive the await; overlapping requests use distinct Context values |
| Greeting inputs | Literal and variable strings, a variable default, and named operation selection |
| Invalid inputs | Missing, null, and incompatible inputs are rejected before resolver invocation; request-error responses omit `data` |
| Domain error response | Exact message and application code, alias-aware path, source location, and `data: null` for the non-null root field |
| Execution after a domain error | A later successful request completes |

The focused check, formatting, and Clippy passed. The observation suite passed three tests and one compile-fail doctest with an explicitly excluded, unchanged Apollo error-code regression. This is feasibility evidence, not product validation or a passing unfiltered prototype suite.

The experiment supports only one directly selected `hello` field against its fixed schema. It rejects repeated fields, fragments, and field directives. It does not establish field collection, general argument processing, nested or list completion, arbitrary extension maps, generated dispatch, HTTP integration, cancellation, or multithreaded execution. Parse/validation diagnostics were aggregated rather than preserving individual locations. In particular, hardcoded root-null handling is not evidence of recursive null propagation or a fix for the nullable-list regression. These experiment limits do not redefine the MVP scope.

### 3.5 Execution and error responsibilities

Generated dispatch knows which Rust resolver to invoke and converts prepared arguments into its generated argument struct. The runtime executor owns GraphQL field collection and merging, applicable fragment/directive evaluation, invocation scheduling, and result completion according to the schema. Settle concrete Rust signatures while implementing this boundary, rather than adding an interchangeable executor-backend abstraction.

Resolvers return a NecrassRs error containing a message and optional extension map. Application codes such as `USER_NOT_FOUND` belong in that map; they are not required by the GraphQL specification. The executor supplies source locations and response paths, including aliases and list indices, collects execution errors, and propagates null to the correct nullable boundary. The HTTP adapter converts the completed result without taking over these responsibilities.

Request errors omit the `data` entry. Execution results contain `data`, which may be partial or null, and include errors when execution fails. Ordinary domain errors must remain separate from the deliberate process-termination policy for selected unimplemented resolvers. Reuse Apollo response types where suitable without exposing Apollo internals in user resolver implementations.

## 4. Crates

| Package | Responsibility | Direct consumers |
| --- | --- | --- |
| `necrassrs` | Public runtime API and integration of request validation, input processing, resolver execution, and response completion | Applications and HTTP adapters |
| `necrassrs-build` | Discover and validate SDL, generate contracts and dispatch, synchronize editable resolver implementations through Rust ASTs, and manage Cargo rebuilds and output | Consumer `build.rs` |
| `necrassrs-axum` | GraphQL HTTP extraction, response conversion, and development UI integration | Axum applications |
| `necrassrs-cli` | The `necrass` binary, initialization templates, and file creation | Developers |

Within `necrassrs-build`, keep contract generation independent of Cargo environment variables and filesystem operations so it can be tested directly. Cargo integration manages inputs, rebuild instructions, disposable output, and reading/writing the designated implementation source. Source synchronization compares Rust ASTs; it does not duplicate Apollo's schema model. The runtime does not depend on the build library.

There is no separate codegen package: it currently has no independent consumer or release lifecycle. A module boundary preserves testability without adding another published dependency. Extract a crate only when another tool needs independent reuse or a concrete dependency or release boundary emerges.

An initial internal layout is sufficient:

```text
necrassrs-build/src/
├── lib.rs       # Public build API and Cargo integration
└── codegen.rs   # Validated schema to Rust code
```

The CLI creates the build-script entry point. Resolver scaffolding and subsequent synchronization are responsibilities of the build library, triggered by Cargo, not a one-time CLI template operation. If later initialization needs schema processing, reuse the library instead of duplicating its logic.

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

A build-time `Schema` instance does not survive into the running server. The generator embeds the validated schema as `generated::SDL` using Apollo's SDL serialization, preserving schema definitions and extensions rather than the original source formatting or comments. Parse and validate it once during application initialization, then reuse that runtime schema across requests. The synchronization acceptance flow edits a generated resolver body and runs that consumer without source SDL files. Do not introduce a separate schema serialization format.

The complete workflow is:

```text
cargo install necrassrs-cli --locked
  → Install the necrass executable
  → necrass init in a consumer project
  → Create build.rs without overwriting an existing file
  → Manually prepare dependencies, SDL, and the application entry point

Edit SDL
  → cargo build
  → build.rs
  → Apollo schema parsing and validation
  → NecrassRs support and Rust naming checks
  → Generate Rust contracts and wiring in OUT_DIR
  → Read the existing resolver implementation AST, or create its initial structs and methods
  → Add new stubs, update retained declarations, and delete removed methods
  → Preserve bodies of retained methods and unrelated user code
  → Compile contracts and synchronized implementations together

Fill generated unimplemented!() bodies with business logic
  → cargo build
  → Preserve those bodies while synchronizing later SDL changes

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

### 7.1 Initial CLI scope

The first `necrass init` scope, tracked in issue #9 under #1, creates only `build.rs` in an existing consumer project. It does not edit Cargo.toml or generate SDL, resolvers, or an Axum server. The consumer supplies dependencies, SDL, and an application entry point; Cargo then invokes the build library to generate and synchronize resolver implementations. Full runnable-project initialization is deferred. Do not add framework selection options or empty templates for other adapters.

The intended installation and initialization flow is shown below. These commands describe the planned product, not an available release:

```sh
cargo install necrassrs-cli --locked
cargo new my-api
cd my-api
necrass init
```

The package is named `necrassrs-cli`; its installed executable is named `necrass`. It is a separately installed development tool, not a consumer `dev-dependency`. Adding a package to `[dev-dependencies]` does not install its executable as a shell command.

The initial CLI does not merge dependencies into an existing `Cargo.toml`. Existing applications follow manual integration instructions. Exact CLI arguments remain open; automatic existing-project integration is deferred until its configuration-preservation behavior is designed.

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
| `schema/**/*.graphql` | User-written public GraphQL contract, discovered recursively |
| `build.rs` | Build-library invocation and input configuration |
| `main.rs` | User routing, handlers, and server configuration |
| `context.rs` | User Context definition |
| `resolvers.rs` | Build-created resolver structs and explicit methods; SDL owns mapped declarations, users own retained method bodies and unrelated state/helpers |
| `generated.rs` | Module that includes generated code from `OUT_DIR` |
| Rust files in `OUT_DIR` | Automatically generated; not edited manually |

Manual setup uses the following dependency locations. A later full-project initializer may write them using a tested release combination; the first CLI command does not:

| Installation or manifest location | Contents |
| --- | --- |
| `cargo install` | `necrassrs-cli`, providing the `necrass` executable |
| `[dependencies]` | `necrassrs`, `necrassrs-axum`, Axum, the async runtime, and other libraries directly used by the application |
| `[build-dependencies]` | `necrassrs-build`, called by `build.rs` |
| `[dev-dependencies]` | Test and example dependencies only when needed; not the CLI |

Ordinary consumers should not need a direct Apollo Compiler dependency. Once initialized, the application builds and runs with Cargo without an installed CLI.

### 7.2 Everyday development

1. Create or choose a consumer Cargo project and prepare its dependencies and entry point. Use `necrass init` to create `build.rs`, or write the build script manually.
2. Define objects, fields, arguments, and nullability in SDL.
3. Run Cargo to generate contracts and wiring and create the editable resolver structs and explicit `unimplemented!()` methods from SDL.
4. Replace generated method bodies with business logic; no handwritten Query or resolver signatures are required to bootstrap.
5. Return user objects through generated wrappers from parent resolvers.
6. Construct Context in the handler and pass it to execution.
7. Run the server and test completed fields.
8. Edit SDL and rebuild to synchronize additions, declaration changes, and deletions with the existing implementation AST.

Constructing a wrapper does not execute child fields. Execution invokes only selected fields. Cargo regeneration must track SDL file and declaration changes. Preserving user code means keeping the bodies of retained methods and unrelated source, not freezing every implementation file byte-for-byte across an SDL change.

### 7.3 Partial implementations and contract changes

The build library generates an editable concrete resolver struct and explicit async trait methods in `src/resolvers.rs`. Every new field starts with an `unimplemented!()` body. Existing trait defaults remain a fallback but do not satisfy this scaffolding requirement. A test that supplies a handwritten Query implementation before its first build does not verify this workflow.

For the first query-only consumer, `build("schema")` creates `src/resolvers.rs`, which the application imports with `mod resolvers`. Contracts and dispatch remain included from `OUT_DIR/necrassrs.rs`. The implementation file must expose the SDL-derived root struct and implement its generated resolver trait, compatible with the application's Context.

Use the injective naming rules in section 3.2 to compare the desired SDL-derived declarations with the existing Rust AST:

| SDL change | Required source synchronization |
| --- | --- |
| No contract change | Preserve existing implementation content; repeated builds must be idempotent |
| Add a field | Add its explicit method with an `unimplemented!()` body |
| Change a retained field's contract | Refresh its Rust declaration and generated Args/type mapping, preserving its method body |
| Delete a field | Remove its method, including any body previously written for that deleted field |
| Rename a field | Delete the old method and add a fresh stub under the new mapped name; never migrate the old body |

Do not infer renames or use fuzzy matching. Preserve application state and unrelated items outside SDL-owned declarations. Parse and validate inputs before destructive synchronization; invalid SDL or an unreadable/unparseable implementation must fail without replacing existing user code. Symlink destinations must not be followed or overwritten.

Retained business logic is not automatically rewritten to accommodate incompatible contract changes. It may require user edits when rustc reports use of removed arguments or an incompatible result. Supported `String!` argument changes can be tested now without expanding scalar support; broader result-type and nullability synchronization must be checked as those types become supported.

This is an explicit source-writing responsibility of the build library. The earlier policy forbidding all build-time edits under `src` is superseded for the designated resolver implementation file only. Other application files remain user-owned. The current implementation still only generates the disposable output; the source synchronization described here remains to be implemented.

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
| Cargo integration and synchronization | Bootstrap from SDL without a supplied resolver implementation; synchronize added/modified/deleted methods; treat renames as delete+add; preserve retained bodies and unrelated user code |
| Public contracts | Compile consumer implementations, diagnose contract changes, allow request borrowing, ensure the entire execution future is `Send` |
| Partial implementation | Do not call unselected fields; execute implemented fields; terminate separate dev and release executables on unimplemented calls |
| GraphQL execution | Preserve input states, variables, defaults, coercion, selection rules, error paths, null propagation, and mutation order |
| Known regression | Nullable list-item conversion errors stop at the correct nullable boundary |
| HTTP | Convert requests/responses, limit bodies, isolate Context, distinguish request and execution errors |
| Initialization and release | Build/run initialized projects, refuse file/symlink overwrites, support packaged dependencies |

Test process termination with separate executable processes, not by catching a panic inside the test runner. When providing GraphiQL, define development introspection settings and UI asset delivery as well.

This table describes target-product validation, not exhaustive GraphQL conformance. The source-synchronization workflow is a maintainer-directed clarification of the build task and supersedes its earlier immutable-user-file interpretation. Document implemented coverage separately from pending acceptance tests and explicit feature restrictions.

### 10.1 MVP implementation work breakdown

The executor direction is recorded in this document; it does not need a separate design-only issue. Register concrete implementation tasks as GitHub sub-issues of #1. The labels below describe proposed tasks, not assigned issue numbers.

| Task | Deliverable and acceptance boundary | Dependencies |
| --- | --- | --- |
| Implement the MVP runtime and execution core | `necrassrs` request, resolver, Context, error, and response contracts plus execution over Apollo models. Verify the greeting behavior with a handwritten test adapter, full-future `Send`, request borrowing, invalid inputs, error paths, and specification-based completion regressions. No production greeting-specific dispatch or hardcoded root-null handling. Establish the workspace as needed. | None |
| Generate resolver contracts and dispatch from SDL | `necrassrs-build` codegen module producing argument structs, resolver contracts, wrappers/dispatch as needed, and embedded SDL. Compile generated code with user implementations; diagnose invalid/unsupported schemas and Rust naming collisions. Verify partial-implementation behavior with the runtime, including subprocess termination checks. | Runtime contracts |
| Integrate generation with Cargo builds | Build-library entry point, SDL discovery, rebuild tracking, `OUT_DIR` contracts, editable resolver scaffolding and AST synchronization, and runtime schema initialization wiring. Test the complete generate/edit/rebuild flow without supplying initial resolver implementations. | Code generation |
| Create the CLI build-script initializer | `necrass init` creates build.rs without overwriting existing entries. Resolver generation and synchronization remain in the build library. | Build API |
| Implement the Axum adapter | `necrassrs-axum` extraction and response conversion with documented methods, media types, status codes, and body limits. Verify user-owned handlers and per-request Context construction through HTTP checks. | Runtime request/response API |
| Deliver the greeting example and integration checks | Real consumer example with hardcoded names, Cargo generation, user resolvers, Axum handler, and runnable instructions. Verify all acceptance criteria of #1 together. | All preceding tasks |

Each task includes its own relevant checks. The final example verifies integration rather than postponing component testing. Code generation and Axum work can proceed independently once their runtime contracts are stable. Minimal CLI build-script creation is tracked under #1; full-project initialization, development UI, and release automation remain deferred.

### 10.2 Type expansion after issue #1

Defer implementation of broader generated type support until the integrated greeting MVP is complete. Organize follow-up issues around these scopes; these are planned work groups, not claims that tracking issues have already been created:

1. Built-in scalars, lists, and nullability across generation, input conversion, and result completion.
2. Enum and input-object representations and conversion, including input presence and recursive inputs.
3. Output objects, interfaces, and unions, including resolver wiring and execution.
4. Custom scalar contracts and input/output conversion.

Existing runtime coverage beyond the generated MVP remains in place. Each expansion must validate the complete supported path rather than declaring support based only on generated Rust types or successful Apollo validation.

## 11. Open decisions

1. **Public resolver and dispatch API:** Concrete Rust signatures, wrapper ownership and lifetimes, internal type erasure if needed, and `Send`/`Sync` bounds. One Context type per schema, argument structs, and the resolver/executor error responsibilities are established.
2. **Execution details:** Field scheduling, recursive completion representation, and generated-dispatch handoff. Ownership of execution and reuse of Apollo validation are established.
3. **Source synchronization implementation:** AST editing and diagnostics that realize section 7.3. Explicit stubs, identity-based updates, retained-body preservation, deletion, and rename-as-delete-plus-add are established requirements, not open product decisions.
4. **Coverage beyond the greeting MVP:** Additional supported types/features, explicit rejection diagnostics, and custom scalar conversion. Do not reopen #1's agreed behavior as an executor-selection task.
5. **HTTP and development UI:** Methods, media types, status codes, introspection settings, and asset distribution.
6. **Release contract:** MSRV, default features, generator/runtime compatibility, and CLI initialization details.

SQL generation, ORM integration, automatic batching, a separate non-`Send` mode, standalone watch, and performance optimization are not prerequisites for this architecture.

## 12. References

- Internal design basis: the 2026-09-17 prototype report, cumulative development article, and subsequent architecture discussions. Do not reproduce private development records in the public repository.
- [Apollo Compiler API](https://docs.rs/apollo-compiler/1.32.0/apollo_compiler/): models, parsing, and validation.
- [GraphQL September 2025: Execution](https://spec.graphql.org/September2025/#sec-Execution): execution semantics.
- [GraphQL September 2025: Errors](https://spec.graphql.org/September2025/#sec-Errors): request and execution error response formats.
- [GraphQL September 2025: Handling Execution Errors](https://spec.graphql.org/September2025/#sec-Handling-Execution-Errors): null propagation.
- [Apollo Compiler changelog](https://github.com/apollographql/apollo-rs/blob/main/crates/apollo-compiler/CHANGELOG.md): versioned features and specification-related fixes.
- [Apollo project and Rust version policy](https://github.com/apollographql/apollo-rs): purpose, license, and support policy.
- [Validation error codes, issue #855](https://github.com/apollographql/apollo-rs/issues/855): diagnostic wording versus programmatic error contracts.
- [Cargo lockfile guidance change](https://blog.rust-lang.org/2023/08/29/committing-lockfiles/): background on committing library lockfiles.
- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html): shared workspace configuration.
- [Cargo dependency locations](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#multiple-locations): local paths, versions, and registry publication.

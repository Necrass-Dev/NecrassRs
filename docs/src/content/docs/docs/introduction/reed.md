---
title: Reed
description: GraphQL parsing, models, validation, and introspection for Rust.
---

Reed is the GraphQL compiler toolkit in this workspace. NecrassRs uses Reed to
parse and validate schemas and operations, but both Reed packages are also
available for direct use in GraphQL tools and applications.

## Reed Parser

[`reed-parser`](https://crates.io/crates/reed-parser) provides lexical analysis,
error-resilient parsing, and typed concrete syntax trees for GraphQL schemas and
executable documents. Use it when a tool needs direct access to GraphQL syntax,
tokens, or source structure.

- [API documentation on docs.rs](https://docs.rs/reed-parser/)
- [Package README and examples](https://github.com/Necrass-Dev/NecrassRs/tree/main/crates/reed-parser)

## Reed Compiler

[`reed-compiler`](https://crates.io/crates/reed-compiler) builds on Reed Parser
with schema and executable-document models, validation, diagnostics,
introspection, and input coercion. Use it when an application needs validated
GraphQL semantics rather than direct concrete-syntax-tree access.

- [API documentation on docs.rs](https://docs.rs/reed-compiler/)
- [Package README and examples](https://github.com/Necrass-Dev/NecrassRs/tree/main/crates/reed-compiler)

NecrassRs re-exports the public Reed types used by its runtime contracts, so an
ordinary NecrassRs application does not need to add a direct Reed dependency.

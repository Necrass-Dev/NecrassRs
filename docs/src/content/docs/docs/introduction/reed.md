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

## Specification baseline

The current source checkout uses GraphQL September 2025 at
[`89d93ebbe05db06787646d76a696ead8de117b2b`](https://github.com/graphql/graphql-spec/tree/89d93ebbe05db06787646d76a696ead8de117b2b)
and Working Draft
[`59bc70ac974b0d5d1e00869f41c06bb3bfc756da`](https://github.com/graphql/graphql-spec/tree/59bc70ac974b0d5d1e00869f41c06bb3bfc756da).
In this checkout, the pinned Draft takes precedence where they differ, and no
runtime edition switch is provided. Future versions may adopt a newer baseline;
updates need an explicit comparison, regression checks, and compatibility notes.
This is draft support, not a claim of complete conformance or support for a later
released edition.

User-visible differences from the released baseline include:

- Empty selection sets are accepted, while composite fields still require a
  selection set and scalar or enum fields must have none.
- Input-cycle validation considers OneOf alternatives as well as ordinary input
  objects. Default-expansion cycles remain a separate validation rule.
- Directive definitions can carry directives and be extended or deprecated.
  Introspection includes `DIRECTIVE_DEFINITION` and directive deprecation metadata.
- Subscription source creation precedes operation execution. Creation failures
  are request errors without `data`; see [Subscriptions](/docs/subscriptions/).

The [conformance tests](https://github.com/Necrass-Dev/NecrassRs/blob/main/crates/reed-compiler/tests/conformance.rs)
and [clause checks](https://github.com/Necrass-Dev/NecrassRs/blob/main/crates/reed-compiler/tests/clause_inventory.rs)
exercise these compiler boundaries. [Issue #18](https://github.com/Necrass-Dev/NecrassRs/issues/18)
tracks integrated conformance work. HTTP and streaming transport behavior is a
separate contract documented in [Integration](/docs/integration/).

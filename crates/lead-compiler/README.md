<div align="center">
  <h1><code>lead-compiler</code></h1>

  <p>
    <strong>A query-based compiler for the GraphQL query language.</strong>
  </p>
  <p>
    <a href="https://crates.io/crates/lead-compiler">
        <img src="https://img.shields.io/crates/v/lead-compiler.svg?style=flat-square" alt="Crates.io version" />
    </a>
    <a href="https://crates.io/crates/lead-compiler">
        <img src="https://img.shields.io/crates/d/lead-compiler.svg?style=flat-square" alt="Download" />
    </a>
    <a href="https://docs.rs/lead-compiler/">
        <img src="https://img.shields.io/static/v1?label=docs&message=lead-compiler&color=blue&style=flat-square" alt="docs.rs docs" />
    </a>
  </p>
</div>

## Features

* A (comparatively) low-level AST for GraphQL grammar,
  and high-level representation of `Schema` and `ExecutableDocument`.
* All three can be parsed (using `lead-parser` internally),
  created or modified programatically,
  and serialized.
* Validation of schemas and executable documents, as defined [in the GraphQL specification][val].
* Execution of the [schema introspection][introsp] portion of queries.

[val]: https://spec.graphql.org/September2025/#sec-Validation
[introsp]: https://spec.graphql.org/September2025/#sec-Introspection

## Getting started
Add the dependency to start using `lead-compiler`:
```bash
cargo add lead-compiler
```

Or add this to your `Cargo.toml` for a manual installation:

```toml
# Just an example, change to the necessary package version.
[dependencies]
lead-compiler = "0.1.0"
```

## Rust versions

`lead-compiler` is tested on the latest stable version of Rust.
Older version may or may not be compatible.

## Usage

You can get started with `lead-compiler`:
```rust
use lead_compiler::Schema;
use lead_compiler::ExecutableDocument;

let sdl = r#"
  type Query {
    field: Int
  }
"#;
let query = "{ field }";

/// In case of validation errors, the panic message will be nicely formatted
/// to point at relevant parts of the source file(s)
let schema = Schema::parse_and_validate(sdl, "sdl.graphql").unwrap();
let doc = ExecutableDocument::parse_and_validate(&schema, query, "query.graphql").unwrap();
```

### Examples
#### Accessing fragment definition field types

```rust
use lead_compiler::{Schema, ExecutableDocument, Node, executable};

let schema_input = r#"
type User {
  id: ID
  name: String
  profilePic(size: Int): URL
}

schema { query: User }

scalar URL @specifiedBy(url: "https://tools.ietf.org/html/rfc3986")
"#;
let query_input = r#"
query getUser {
  ... vipCustomer
}

#fragment definition where we want to know the field types.
fragment vipCustomer on User {
  id
  name
  profilePic(size: 50)
}
"#;

let schema = Schema::parse_and_validate(schema_input, "schema.graphql").unwrap();
let document = ExecutableDocument::parse_and_validate(&schema, query_input, "query.graphql")
    .unwrap();

let op = document.operations.get(Some("getUser")).expect("getUser query does not exist");
let fragment_in_op = op.selection_set.selections.iter().filter_map(|sel| match sel {
    executable::Selection::FragmentSpread(spread) => {
        Some(document.fragments.get(&spread.fragment_name)?.as_ref())
    }
    _ => None
}).collect::<Vec<&executable::Fragment>>();

let fragment_fields = fragment_in_op.iter().flat_map(|frag| {
    frag.selection_set.fields()
}).collect::<Vec<&Node<executable::Field>>>();
let field_ty = fragment_fields
    .iter()
    .map(|f| f.ty().inner_named_type().as_str())
    .collect::<Vec<&str>>();
assert_eq!(field_ty, ["ID", "String", "URL"]);
```

#### Get a directive defined on a field used in a query operation definition.
```rust
use lead_compiler::{Schema, ExecutableDocument, Node, executable};

let schema_input = r#"
type Query {
  topProducts: Product
}

type Product {
  inStock: Boolean @join__field(graph: INVENTORY)
  name: String @join__field(graph: PRODUCTS)
}

enum join__Graph {
  INVENTORY,
  PRODUCTS,
}
directive @join__field(graph: join__Graph) on FIELD_DEFINITION
"#;
let query_input = r#"
query getProduct {
  topProducts {
    inStock
  }
}
"#;

let schema = Schema::parse_and_validate(schema_input, "schema.graphql").unwrap();
let document = ExecutableDocument::parse_and_validate(&schema, query_input, "query.graphql")
    .unwrap();

let get_product_op = document
    .operations
    .get(Some("getProduct"))
    .expect("getProduct query does not exist");

let in_stock_field = &get_product_op
    .selection_set
    .fields()
    .find(|f| f.name == "topProducts")
    .expect("topProducts field does not exist")
    .selection_set
    .fields()
    .find(|f| f.name == "inStock")
    .expect("inStock field does not exist")
    .definition;
let in_stock_directive: Vec<_> = in_stock_field
    .directives
    .iter()
    .map(|dir| &dir.name)
    .collect();
assert_eq!(in_stock_directive, ["join__field"]);
```

#### Printing diagnostics for a faulty GraphQL document
```rust
use lead_compiler::parser::Parser;

let input = "{ ... }";

if let Err(diagnostics) = Parser::new().parse_mixed_validate(input, "document.graphql") {
    println!("{diagnostics}")
}
```

## License
Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

## Upstream provenance

Lead Compiler 0.1.0 is derived from Apollo Compiler 1.33.0 at upstream revision
[`5ebcc40bdf6919843c122e9a76b241689edcd6ef`](https://github.com/apollographql/apollo-rs/tree/5ebcc40bdf6919843c122e9a76b241689edcd6ef/crates/apollo-compiler).
NecrassRs maintains the source corrections recorded in
[`patches/apollo-compiler-1.33.0.patch`](../../patches/apollo-compiler-1.33.0.patch).
Package and crate paths, the parser dependency, documentation, workspace rustfmt
layout, and regression tests are adapted for Lead; the compiler public API and
module organization are retained.

[`salsa`]: https://docs.rs/salsa/latest/salsa/

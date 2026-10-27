---
title: Custom Scalars
description: Define application-owned scalar storage, input coercion, output serialization, and regeneration behavior.
---

GraphQL custom scalar declarations name a type but do not define its Rust
representation or conversion rules. NecrassRs generates an editable,
string-backed starting point for each custom scalar. Your application owns that
type and decides which GraphQL literals, JSON variable values, and response
values are valid.

## Declare a scalar in SDL

Declare the scalar and use it like any other GraphQL input or result type:

```graphql
scalar DateTime @specifiedBy(url: "https://scalars.graphql.org/andimarek/date-time.html")

type Query {
  echo(value: DateTime!): DateTime!
}
```

The optional `@specifiedBy` URL is exposed as `specifiedByURL` through
introspection. NecrassRs does not fetch the URL or infer validation from the
scalar name or specification.

## Build the scalar scaffold

Run `cargo build` after adding the scalar. By default, NecrassRs creates
`src/resolvers/scalars.rs` and re-exports the type from `src/resolvers.rs`:

```text
src/
├── resolvers.rs
└── resolvers/
    └── scalars.rs
```

The generated scaffold accepts GraphQL string literals and JSON string values
and serializes the stored string as JSON:

```rust title="src/resolvers/scalars.rs"
pub struct DateTime(::std::string::String);

impl ::necrassrs::Scalar for DateTime {
    fn parse_literal(
        value: &::necrassrs::ScalarLiteral,
    ) -> ::core::result::Result<Self, ::necrassrs::ScalarError> {
        match value {
            ::necrassrs::ScalarLiteral::String(value) => {
                ::core::result::Result::Ok(Self(value.to_owned()))
            }
            _ => ::core::result::Result::Err(::necrassrs::ScalarError::new(
                "expected a string literal",
            )),
        }
    }

    fn parse_value(
        value: &::necrassrs::JsonValue,
    ) -> ::core::result::Result<Self, ::necrassrs::ScalarError> {
        value
            .as_str()
            .map(|value| Self(value.to_owned()))
            .ok_or_else(|| ::necrassrs::ScalarError::new("expected a string value"))
    }

    fn serialize(
        &self,
    ) -> ::core::result::Result<::necrassrs::JsonValue, ::necrassrs::ScalarError> {
        ::core::result::Result::Ok(self.0.as_str().into())
    }
}
```

This scaffold is not a `DateTime` validator. Change its representation and
conversion bodies when the scalar needs format validation, a numeric transport
value, or another application type. Generated argument and result contracts use
the stable path `crate::resolvers::DateTime`.

## Define the conversion paths

The `necrassrs::Scalar` trait separates the three GraphQL boundaries:

| Method          | Receives                    | Used for                                             |
| --------------- | --------------------------- | ---------------------------------------------------- |
| `parse_literal` | `&necrassrs::ScalarLiteral` | GraphQL literals and GraphQL or SDL default literals |
| `parse_value`   | `&necrassrs::JsonValue`     | Values explicitly supplied in the JSON variables map |
| `serialize`     | `&self`                     | Resolver results written to the GraphQL response     |

Literal kinds are preserved until `parse_literal` runs. A string literal, enum
literal, integer literal, and floating-point literal therefore remain distinct.
Custom integer literals are not first restricted to the JSON number range, so a
scalar may match `ScalarLiteral::Int(value)` and retain an arbitrarily large
integer with `value.as_str()`.

Defaults keep their literal origin. This includes operation variable defaults
and SDL input-field defaults applied while coercing a JSON variable object. A
value explicitly present in the JSON variables map uses `parse_value`; a default
inserted because a value is absent uses `parse_literal`.

NecrassRs owns nullability, omission, and list coercion around the scalar. Your
implementation only converts one non-null scalar value at a time.

For the `echo` field above, the generated argument's `value` field and resolver
output are both the application-owned `DateTime` type. Returning `args.value`
therefore echoes the already converted value; add ordinary constructors or
accessors to the scalar implementation when application logic needs them.

## Handle conversion errors

Return `necrassrs::ScalarError::new(message)` when a conversion fails.

- Invalid custom scalar variables are rejected during variable coercion, before
  any field resolver runs.
- Invalid argument literals become field execution errors with the applicable
  response path and null propagation.
- `serialize` failures are execution errors and follow the field's nullability.

Conversion methods must be deterministic and side-effect-free. NecrassRs does
not guarantee how many times a value is converted; a validated variable may be
converted again when the owned resolver argument is constructed.

## Regenerate safely

Later builds append scaffolds for new scalars while preserving edited scalar
types and conversion bodies. Removing or renaming a scalar deletes its old
scaffold and re-export only when they are still untouched generated declarations.
An edited stale scalar remains application code and must be removed manually.

NecrassRs normally uses the internal module name `scalars`. If an SDL type maps
to that Rust name, synchronization chooses a collision-free internal module name
while keeping the public scalar path under `crate::resolvers` unchanged. Scalar
names that collide with names such as `Result`, `Sync`, `std`, or `necrassrs`
also compile because generated scaffolds use qualified framework and standard
library paths.

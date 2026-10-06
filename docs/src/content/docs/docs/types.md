---
title: Types
description: GraphQL-to-Rust type mappings, input presence, defaults, OneOf, and recursive inputs.
---

NecrassRs generates Rust argument and result types from your GraphQL SDL. This
page describes those types and the values a resolver receives. For a complete
web server example, follow the [Tutorial](/docs/tutorial/).

The generated API currently supports built-in scalar and enum results, ordinary
and OneOf input objects, nullable and list forms, and recursive inputs. Custom
scalars, composite results, and generated mutation/subscription routing remain
unsupported.

## Rust mappings

| SDL                   | Rust                             |
| --------------------- | -------------------------------- |
| `Int!`                | `i32`                            |
| `Float!`              | `f64`                            |
| `String!`             | `String`                         |
| `Boolean!`            | `bool`                           |
| `ID!`                 | `necrassrs::Id`                  |
| `Status!`             | `generated::types::Status`       |
| Nullable input `Int`  | `GraphQLInput<i32>`              |
| Nullable result `Int` | `Option<i32>`                    |
| Input `[Int]`         | `GraphQLInput<Vec<Option<i32>>>` |
| Input `[Int]!`        | `Vec<Option<i32>>`               |
| Input `[Int!]`        | `GraphQLInput<Vec<i32>>`         |
| Input `[Int!]!`       | `Vec<i32>`                       |
| Result `[Int]`        | `Option<Vec<Option<i32>>>`       |
| Result `[Int]!`       | `Vec<Option<i32>>`               |
| Result `[Int!]`       | `Option<Vec<i32>>`               |
| Result `[Int!]!`      | `Vec<i32>`                       |

`GraphQLInput` is available from `necrassrs`. Generated field argument structs
live at `generated::types::<Object>::<field>::Args`; generated enums and input
objects live under `generated::types`.

List items have no undefined state. Input singleton coercion applies recursively:
for `[[Int]]`, the input `3` becomes `[[3]]`. Enum literals are unquoted, as in
`status: CLOSED`; JSON enum variables use strings, such as `{"status":"CLOSED"}`.
Quoted GraphQL enum literals and unknown enum members are rejected.

`Id` preserves supplied string contents, including leading zeros, and serializes
results as strings. Integer inputs are accepted. `Int` enforces signed 32-bit
bounds. JSON variable numbers with an empty fractional part are treated as
integers; a GraphQL floating-point literal remains a different literal kind.
Non-finite Float results produce execution errors; nullable list items preserve
valid siblings and report the failed item's index.

## GraphQLInput

Nullable arguments and ordinary nullable input-object fields use the
`necrassrs::GraphQLInput<T>` enum:

```rust
pub enum GraphQLInput<T> {
    Undefined,
    Null,
    Value(T),
}
```

`Undefined` means the input was omitted and no applicable default supplied a
value. `Null` means the caller explicitly supplied null. `Value(T)` contains a
non-null value. This distinction lets resolvers treat an omitted input differently
from an explicit null, which `Option<T>` alone cannot represent.

These states describe the value **after defaults and coercion**, not the original
request spelling. Nullable results use `Option<T>` because output values have no
undefined state.

### Input presence and defaults

Using the `Filter` and `statuses` definitions from the tutorial:

| Input                                  | Resolver value                                          |
| -------------------------------------- | ------------------------------------------------------- |
| Omit `filter`                          | `Value(Filter { ... })`, from the argument default `{}` |
| Omit `status` inside a supplied filter | `Value(Status::OPEN)`, from the field default           |
| Supply `status: null`                  | `Null`; the default does not replace explicit null      |
| Omit `limit`                           | `Undefined`, because it has no default                  |
| Supply `limit: null`                   | `Null`                                                  |
| Supply `limit: 5`                      | `Value(5)`                                              |

A variable default applies when that variable is absent. An explicit null variable
does not activate its default. An absent argument variable permits an applicable
argument default. Unknown input fields are rejected before resolver dispatch.

## OneOf inputs

The tutorial's `Locator` input is generated as an enum with `id(Id)` and
`name(String)` variants. The selected payload has no `GraphQLInput` wrapper.
Exactly one non-null entry is required: `{id: "001"}` is valid, while `{}`,
`{id: null}`, and `{id: "001", name: "Sheri"}` are invalid. OneOf schema fields
cannot be non-null or have defaults. A variable used directly as a member value
must have a compatible non-null variable type.

## Recursive inputs

In the tutorial's `Filter`, `next` uses `GraphQLInput<Box<Filter>>`; `children`
uses `GraphQLInput<Vec<Filter>>`. Lists already provide indirection. The generator
boxes nullable singular edges within recursive components, including mutual
recursion and OneOf payloads. Non-null and non-cyclic edges stay inline. Recursive
OneOf payloads use `Box<T>`, without another presence wrapper. GraphQL schema and
default-cycle validation are separate from this Rust layout analysis.

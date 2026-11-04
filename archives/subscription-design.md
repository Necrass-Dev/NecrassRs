# Subscription contracts and RED acceptance tests

Status: accepted design for [issue #29](https://github.com/Necrass-Dev/NecrassRs/issues/29),
not implemented. This change adds tests and documentation only. Production
runtime, code generation, and synchronization remain unchanged.

## Specification boundary

Use the baseline in [specs.md](specs.md): September 2025 plus the immutable
Working Draft `59bc70ac974b0d5d1e00869f41c06bb3bfc756da`, with the pinned Draft
taking precedence. Source creation is part of request preparation; its failures
are request errors without `data`. Each source event produces an execution
result. Execution errors, including root-null results, do not terminate the
subscription. Source failure is terminal; response cancellation cancels the
source. Internal exceptional execution failures cancel the source and terminate
the response stream with an error.

References: [pinned execution algorithms](https://github.com/graphql/graphql-spec/blob/59bc70ac974b0d5d1e00869f41c06bb3bfc756da/spec/Section%206%20--%20Execution.md)
and [single-root validation](https://github.com/graphql/graphql-spec/blob/59bc70ac974b0d5d1e00869f41c06bb3bfc756da/spec/Section%205%20--%20Validation.md#single-root-field).

## Public contracts

User field implementations use `resolve` consistently. Query, Mutation, and
Object fields retain `Resolver::resolve`. Subscription root fields implement
`Subscriber::resolve`, called once to create the source. Its associated `Output`
is one SDL field value, not the concrete stream type.

```rust
pub trait Subscriber<F, C>: Send + Sync
where
    F: Field,
    C: Sync,
{
    type Output: Send;

    fn resolve<'a>(
        &'a self,
        context: &'a C,
        args: F::Args,
    ) -> impl Future<Output = Result<SourceStream<'a, Self::Output>, ResolverError>>
    + Send + 'a;
}

pub enum SourceEvent<T> {
    Value(T),
    FieldError(ResolverError),
    SourceError(ResolverError),
}

pub enum SubscriptionError {
    Source(ResolverError),
    Internal,
}
```

`SourceStream<'a, T>` has private storage and a `from_stream` constructor accepting
`Stream<Item = SourceEvent<T>> + Send + 'a`, for `T: Send + 'a`. The input need
not be `Sync`, `Unpin`, or `'static`. The wrapper implements `Stream` with the
same item type, `Send`, and `Unpin`. After emitting a source error, or observing
normal exhaustion, it releases the input and returns `None` on subsequent polls.
`FieldError` represents a field failure during an event, not failure of the
source stream itself.

`ResponseStream<'a>` is created only by the runtime. It implements
`Stream<Item = Result<Response, SubscriptionError>> + Send + Unpin + 'a`.
`Ok(Response)` always represents an execution result, including field errors or
`data: null`. A terminal `Err` is emitted once; later polls return `None`.
Termination releases resources without requiring another poll or drop.
`SubscriptionError` supports `Debug`, `Display`, and `std::error::Error`.
Its public shape does not expose internal scheduling or diagnostic types.
Panics are not automatically converted to `Internal`.

`subscribe(schema, request, dispatcher, context)` returns
`Result<ResponseStream<'a>, Response>` asynchronously. Only request-error
responses appear in the outer `Err`. Source creation finishes before `Ok` is
returned; it is not deferred until the first response poll. The entire startup
future is `Send`. `subscribe_with_options` also accepts existing
`ExecutionOptions` as its final argument. Both borrow schema, dispatcher, and
Context for `'a`; the original `Request` can be dropped after startup.

The low-level `Dispatcher<C>` adds this method, with a default unsupported-source
error so existing handwritten dispatchers remain valid:

```rust
fn resolve_event_stream<'stream, 'call>(
    &'stream self,
    context: &'stream C,
    coordinate: FieldCoordinate<'call>,
    arguments: &'call JsonMap,
    argument_literals: &'call ScalarLiteralMap,
) -> impl Future<
    Output = Result<SourceStream<'stream, ResolvedValue<C>>, ResolverError>,
> + Send + 'call
where
    'stream: 'call;
```

Temporary argument maps and coordinates live only for the creation call.
Generated dispatch converts them to owned `Args` and invokes
`Subscriber::resolve`. Scalar literal kinds and nested conversion errors must
survive this boundary. Generated `SchemaDispatcher::new` takes declared roots
in Query, Mutation, Subscription order, omitting absent roots without placeholders.

## Execution and synchronization

- Prepare and validate the request once, coerce variables, and create the source
  once. Reuse the prepared document and variables without removing required
  field-argument coercion or result completion.
- Process events sequentially within a subscription, without prefetching the
  next event while completing the current one. This is a project policy, not a
  mandated GraphQL scheduler.
- Keep existing concurrent sibling-field and list-item completion, response
  ordering, and serial mutation-root behavior. Separate subscriptions progress
  independently.
- Treat each supplied field value as the selected subscription root field's
  result. Preserve aliases, merged selections, locations, indexed error paths,
  abstract runtime types, scalar coercion, and null propagation. Errors from one
  event must not leak into the next.
- Drop/cancellation releases the source and active event futures; cancellation
  during source creation releases its in-progress future. No default detached
  tasks or unbounded queues are introduced. External producers own their
  buffering, loss, and cancellation integration.
- Reuse [architecture section 7.3](architecture.md#73-partial-implementations-and-contract-changes)
  for naming and synchronization. Preserve retained bodies/bindings, add stubs,
  protect edited stale declarations, and treat renames as deletion plus addition.
  Follow reachable modules and validate the full plan before writes.
- Applications own Context. Borrowed source streams do not imply support for
  borrowed Object outputs. No Context factory, automatic replay/reconnect,
  acknowledgement, or transport framing is added.

## RED test layout

[Cargo consumer cases](../crates/necrassrs-build/tests/cases/subscriptions.rs)
reuse the existing `Consumer` harness. All acceptance tests assert successful
compilation/build/execution; missing APIs are failures, not expected-error
snapshots or ignored tests.

| Area | Fixture and checks |
| --- | --- |
| Runtime API and lifetimes | [Runtime consumer](../crates/necrassrs-build/tests/fixtures/subscription_runtime.rs): `Send`, borrowing, non-`Sync`/non-`Unpin` inputs, temporary argument maps, Request release, legacy Dispatcher compatibility |
| G19 / G17 | Invalid requests invoke no source or fields; source creation errors omit `data`; operation selection, variables, nested directives, and execution options |
| G20 / G17 | Merged selections, aliases, event parents, field and root-null errors followed by success, scalar/list completion, error metadata and isolation |
| Scheduling | Explicit pending/release polls prove concurrent siblings/list items, serial events, and independent subscriptions without timing comparisons |
| G21 | Normal exhaustion, terminal source failure, no subsequent source polls, immediate resource release, cancellation before/while reading events and during field execution/source creation |
| Generated consumer | [SDL](../crates/necrassrs-build/tests/fixtures/subscription/schema.graphql), [resolvers](../crates/necrassrs-build/tests/fixtures/subscription/resolvers.rs), [execution checks](../crates/necrassrs-build/tests/fixtures/subscription/main.rs): typed inputs, custom scalar literal/variable distinction, owned and abstract outputs, existing query/mutation routing |
| Source synchronization | Cold bootstrap, both subscription constructor forms, custom roots/Context, injective naming, retained bodies/output changes, additions, safe deletion/rename, edited-declaration protection, removed-argument protection, moved implementations |

The runtime consumer disables code generation only in its temporary test crate
so generator rejection does not mask the independent missing-runtime-API RED.
The generated consumer first bootstraps from SDL without handwritten resolvers,
then supplies application implementations and executes their contracts.

Current blockers are missing runtime subscription APIs and generator rejection
of subscription roots. Behavioral assertions remain unexecuted behind these
compile/build failures. Fixture syntax checks are not compilation or conformance
evidence. Internal exceptional-error cleanup still needs a reproducible runtime
failure case when an implementation exists; do not introduce a public fault
injection API or convert arbitrary panics merely to exercise it. Transport
exchanges, load/performance measurements, and full subscription conformance
remain outside this RED checkpoint.

## Validation at this checkpoint

- `cargo fmt --all -- --check`: passed. Standalone fixture Rust files also passed
  an explicit `rustfmt --check --edition 2024` invocation because `include_str!`
  does not make them Cargo formatting targets.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
  This checks the harness, not the not-yet-compilable consumer fixtures.
- `cargo test --workspace --locked`: failed at `cargo_build` with exactly the
  nine new RED acceptance cases. Its other 38 tests passed (37 existing cases
  and the new fixture-syntax check). Earlier test targets passed; Cargo stopped
  before later workspace targets and doctests.
- Eight RED cases stop at `Generated dispatch does not support subscription
  roots`. The independent runtime case stops at absent subscription exports and
  the absent `Dispatcher::resolve_event_stream` method (E0432/E0407/E0599).
  Its 16 runtime checks and the generated consumer's three execution checks
  have not executed. Their eventual type checking may expose further fixture
  issues; no temporary implementation was added to make them compile.
- No production source, dependency manifest, or root lockfile was changed.

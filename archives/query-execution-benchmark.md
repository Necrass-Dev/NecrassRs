# Query Execution Benchmark Baseline

Date: 2026-10-20

This benchmark compares NecrassRs 0.1.0 with async-graphql 7.2.1 under equivalent
immediate and simulated asynchronous resolver workloads. It establishes a baseline
for issue #58; it does not change execution scheduling or recommend a particular
concurrency implementation.

## Reproduction

Run the benchmark from the repository root:

```sh
cargo bench -p necrassrs --bench query_execution
```

Compile it without measuring:

```sh
cargo bench -p necrassrs --bench query_execution --no-run
```

The benchmark uses this schema:

```graphql
type Query {
  immediate(value: Int!): Int!
  delayed(value: Int!): Int!
  nested: Nested!
  items(count: Int!): [Item!]!
  chain: Chain!
}

type Nested {
  delayed(value: Int!): Int!
}

type Item {
  value: Int!
}

type Chain {
  next: Chain!
  value: Int!
}
```

## Method

- Both implementations run in the same process on the same current-thread Tokio
  runtime.
- Schema and fixture construction occur before measurement. Each generated query is
  executed once against both implementations and its complete response is checked
  before Criterion measures it.
- The timed region includes GraphQL request parsing, validation, field execution,
  completion, and the call to `Runtime::block_on`. It excludes fixture construction
  and the one-time response comparison.
- Immediate fields return their integer argument without a simulated delay.
- Delayed fields wait with `tokio::time::sleep(Duration::from_millis(2))`. This is a
  minimum simulated asynchronous delay, not 2 ms of CPU work or an exact 2 ms wall
  clock duration. A single delayed field took about 3.8 ms on this machine because
  timer resolution, wake-up scheduling, and GraphQL execution are also measured.
- Root and nested cases select 1, 4, or 16 alias-distinct independent fields. List
  cases return 1, 4, or 16 items whose `value` fields wait independently.
- The deep-path case selects `next` eight times followed by `value`. Its nine delayed
  resolver calls depend on the preceding result and therefore cannot overlap.
- The async-graphql fixture uses static `#[Object]` types. In async-graphql 7.2.1,
  its normal container and list completion paths schedule independent values
  concurrently; see the upstream [container execution][container] and [list
  completion][list] implementations.
- Telemetry, tracing subscribers, DataLoader, HTTP handling, databases, and external
  I/O are disabled.

[container]: https://github.com/async-graphql/async-graphql/blob/v7.2.1/src/resolver_utils/container.rs
[list]: https://github.com/async-graphql/async-graphql/blob/v7.2.1/src/resolver_utils/list.rs

## Environment

| Component | Value |
| --- | --- |
| Operating system | macOS 27.0.1 (Build 26A434), arm64 |
| Processor | Apple M4 Pro, 12 cores (8 performance and 4 efficiency) |
| Memory | 24 GB |
| Rust | rustc 1.96.1 (31fca3adb 2026-06-26) |
| Cargo | cargo 1.96.1 (356927216 2026-06-26) |
| Benchmark profile | Cargo `bench` profile (optimized) |
| Executor revision | `57f2f696167bd1f3fec6511e530f5efef3b14ee1` |
| async-graphql | 7.2.1, exact version pinned |
| Criterion | 0.7.0 |
| Tokio | 1.53.1 |

The executor revision identifies the source baseline used for measurement. The
working tree contained only the benchmark, dependency, ignore-rule, and result-note
changes described here; the executor implementation was unchanged.

## Results

Values are Criterion mean estimates with 95% confidence intervals. Immediate-field
results are in microseconds; delayed results are in milliseconds.

### Immediate root fields

| Selected fields | NecrassRs | async-graphql |
| ---: | ---: | ---: |
| 1 | 3.866 µs (3.860–3.871) | 3.795 µs (3.788–3.802) |
| 4 | 9.748 µs (9.727–9.766) | 9.108 µs (9.096–9.120) |
| 16 | 32.347 µs (32.293–32.397) | 30.143 µs (30.097–30.188) |

Both implementations remain in the same order of magnitude as the selection grows.
These results isolate framework work from the simulated asynchronous wait; they do
not establish general framework superiority.

### Delayed root fields

| Selected fields | NecrassRs | async-graphql |
| ---: | ---: | ---: |
| 1 | 3.764 ms (3.746–3.783) | 3.786 ms (3.765–3.807) |
| 4 | 15.048 ms (14.958–15.143) | 3.818 ms (3.801–3.836) |
| 16 | 59.672 ms (59.412–59.949) | 3.882 ms (3.864–3.901) |

### Delayed nested sibling fields

| Selected fields | NecrassRs | async-graphql |
| ---: | ---: | ---: |
| 1 | 3.784 ms (3.762–3.809) | 3.787 ms (3.769–3.806) |
| 4 | 15.014 ms (14.953–15.079) | 3.814 ms (3.794–3.835) |
| 16 | 59.831 ms (59.488–60.252) | 3.921 ms (3.900–3.943) |

### Delayed list item fields

| List items | NecrassRs | async-graphql |
| ---: | ---: | ---: |
| 1 | 3.781 ms (3.765–3.798) | 3.794 ms (3.772–3.819) |
| 4 | 15.046 ms (14.995–15.099) | 3.804 ms (3.783–3.827) |
| 16 | 59.394 ms (59.127–59.666) | 3.834 ms (3.813–3.857) |

### Delayed deep path

| Nested `next` fields | Delayed resolver calls | NecrassRs | async-graphql |
| ---: | ---: | ---: | ---: |
| 8 | 9 | 33.608 ms (33.498–33.717) | 33.668 ms (33.513–33.833) |

## Interpretation

For independent delayed root fields, nested sibling fields, and list item fields,
NecrassRs latency grows approximately with the number of delayed resolver calls.
async-graphql stays near the cost of one observed timer wait as the independent work
increases. At baseline revision `57f2f696167bd1f3fec6511e530f5efef3b14ee1`,
this is evidence that NecrassRs awaited these independent values sequentially while
the measured async-graphql paths overlapped their waits.

The deep path takes the same time in both implementations because each resolver
depends on the object returned by the preceding resolver. Concurrent scheduling
cannot reduce that dependency chain.

## Post-change comparison

The benchmark was rerun after revision
`7f07045d525a075272e71b9b39307c95f76e1787` changed query-root fields, nested
sibling fields, and list items to concurrent polling within the request future.
The following values are the Criterion point estimates from that run on the same
environment. Baseline values are repeated from the tables above.

| Workload | Independent values | Sequential baseline | Concurrent executor |
| --- | ---: | ---: | ---: |
| Delayed root fields | 4 | 15.048 ms | 3.770 ms |
| Delayed root fields | 16 | 59.672 ms | 3.943 ms |
| Delayed nested sibling fields | 4 | 15.014 ms | 3.839 ms |
| Delayed nested sibling fields | 16 | 59.831 ms | 3.954 ms |
| Delayed list item fields | 4 | 15.046 ms | 3.791 ms |
| Delayed list item fields | 16 | 59.394 ms | 3.876 ms |
| Delayed deep path | 9 calls | 33.608 ms | 33.789 ms |

The independent delayed workloads stay near one observed timer wait after the
change instead of growing with the number of selected values. The dependent deep
path is effectively unchanged. This supports the conclusion that the new executor
overlaps independent asynchronous waits without shortening work whose results form
a dependency chain. It does not imply per-field task spawning, a global scheduler,
or general framework superiority.

## Limitations

This is a synthetic latency benchmark on one Apple Silicon machine using a
current-thread runtime. It does not measure HTTP handling, database pools,
transactions, lock contention, DataLoader behavior, CPU-bound parallelism,
multi-threaded execution, throughput under concurrent requests, or production tail
latency. Real applications may constrain otherwise independent work through a shared
connection, transaction, resource limit, or downstream service.

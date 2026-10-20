use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use necrassrs::Request;
use serde_json::json;
use tokio::runtime::Builder;

mod async_graphql_case;
mod necrassrs_case;
mod workload;

use async_graphql_case::Fixture as AsyncGraphqlFixture;
use necrassrs_case::Fixture as NecrassrsFixture;
use workload::{Case, Workload};

fn query_execution(criterion: &mut Criterion) {
    let runtime = Builder::new_current_thread().enable_time().build().unwrap();
    let necrassrs = NecrassrsFixture::new();
    let async_graphql = AsyncGraphqlFixture::new();

    for workload in Workload::ALL {
        let mut group = criterion.benchmark_group(workload.group_name());

        for &size in workload.sizes() {
            let case = Case::new(workload, size);
            let request = Request::new(case.query.clone());
            let size_label = workload.size_label(size);

            validate_responses(&runtime, &necrassrs, &async_graphql, &request, &case);

            group.bench_with_input(
                BenchmarkId::new("necrassrs", &size_label),
                &size,
                |benchmark, _| {
                    benchmark.iter(|| black_box(runtime.block_on(necrassrs.execute(&request))));
                },
            );
            group.bench_with_input(
                BenchmarkId::new("async_graphql", &size_label),
                &size,
                |benchmark, _| {
                    benchmark
                        .iter(|| black_box(runtime.block_on(async_graphql.execute(&case.query))));
                },
            );
        }

        group.finish();
    }
}

fn validate_responses(
    runtime: &tokio::runtime::Runtime,
    necrassrs: &NecrassrsFixture,
    async_graphql: &AsyncGraphqlFixture,
    request: &Request,
    case: &Case,
) {
    let necrassrs_response = runtime.block_on(necrassrs.execute(request));
    assert_eq!(
        serde_json::to_value(necrassrs_response).unwrap(),
        json!({ "data": case.expected_data })
    );

    let async_graphql_response = runtime.block_on(async_graphql.execute(&case.query));
    assert!(
        async_graphql_response.errors.is_empty(),
        "async-graphql returned errors: {:?}",
        async_graphql_response.errors
    );
    assert_eq!(
        serde_json::to_value(async_graphql_response.data).unwrap(),
        case.expected_data
    );
}

criterion_group!(benches, query_execution);
criterion_main!(benches);

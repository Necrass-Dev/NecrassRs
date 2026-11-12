use necrassrs::{ExecutionOptions, Request, execute, execute_with_options};
use opentelemetry::{
    Value,
    trace::{SpanKind, Status},
};
use serde_json::json;
use tracing::Instrument;

use super::{
    fixture::{Context, Query, schema},
    support::{Capture, attr, child_of, one},
};

#[tokio::test]
async fn anonymous_operation_has_no_invented_name_or_error() {
    let capture = Capture::default();
    let response = capture
        .run(execute(
            &schema(),
            &Request::new("{ nullable }"),
            &Query,
            &Context::default(),
        ))
        .await;
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({"data": {"nullable": null}})
    );
    let spans = capture.spans();
    let operation = one(&spans, "query");
    assert_eq!(operation.span_kind, SpanKind::Server);
    assert_eq!(operation.status, Status::Unset);
    assert_eq!(
        attr(operation, "graphql.operation.type"),
        Some(&Value::from("query"))
    );
    assert_eq!(attr(operation, "graphql.operation.name"), None);
    assert_eq!(attr(operation, "error.type"), None);
    assert_eq!(attr(operation, "necrassrs.cancelled"), None);
}

#[tokio::test]
async fn selected_mutation_uses_its_own_name_and_preserves_serial_execution() {
    let capture = Capture::default();
    let context = Context::default();
    let request =
        Request::new("query Other { ping } mutation Change { first { value } second { value } }")
            .with_operation_name("Change");
    let response = capture
        .run(execute(&schema(), &request, &Query, &context))
        .await;
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({"data": {"first": {"value": 1}, "second": {"value": 1}}})
    );
    assert_eq!(*context.order.lock().unwrap(), ["first", "second"]);
    let spans = capture.spans();
    let operation = one(&spans, "mutation");
    assert_eq!(
        attr(operation, "graphql.operation.name"),
        Some(&Value::from("Change"))
    );
    assert_eq!(
        attr(operation, "graphql.operation.type"),
        Some(&Value::from("mutation"))
    );
    assert_eq!(operation.span_kind, SpanKind::Server);
    assert_eq!(operation.status, Status::Unset);
    let first = one(&spans, "Mutation.first");
    let second = one(&spans, "Mutation.second");
    assert!(
        first.end_time <= second.start_time,
        "mutation field completion must remain serial"
    );
}

async fn request_failure(request: Request, phases: &[&str], unidentified: bool) {
    let capture = Capture::default();
    let context = Context::default();
    let response = capture
        .run(execute(&schema(), &request, &Query, &context))
        .await;
    assert!(response.is_request_error());
    assert!(context.order.lock().unwrap().is_empty());
    let spans = capture.spans();
    let operation = spans
        .iter()
        .find(|span| span.span_kind == SpanKind::Server)
        .expect("request failures must export an operation span");
    assert_eq!(
        spans
            .iter()
            .filter(|span| span.span_kind == SpanKind::Server)
            .count(),
        1
    );
    assert!(matches!(operation.status, Status::Error { .. }));
    assert_eq!(
        attr(operation, "error.type"),
        Some(&Value::from("necrassrs.request_error"))
    );
    if unidentified {
        assert_eq!(operation.name, "GraphQL Operation");
        assert_eq!(attr(operation, "graphql.operation.name"), None);
        assert_eq!(attr(operation, "graphql.operation.type"), None);
    }
    for name in phases {
        let phase = one(&spans, name);
        assert_eq!(phase.span_kind, SpanKind::Internal);
        child_of(phase, operation);
    }
    assert!(!spans.iter().any(|span| span.name == "necrassrs.execute"));
    assert!(
        spans.iter().any(|span| !span.events.is_empty()),
        "request errors need an origin event"
    );
}

#[tokio::test]
async fn syntax_failure_does_not_trust_supplied_operation_name() {
    request_failure(
        Request::new("query {").with_operation_name("Unverified"),
        &["necrassrs.parse"],
        true,
    )
    .await;
}

#[tokio::test]
async fn validation_failure_is_recorded_before_execution() {
    request_failure(
        Request::new("query Invalid { unknown }"),
        &["necrassrs.parse", "necrassrs.validate"],
        false,
    )
    .await;
}

#[tokio::test]
async fn ambiguous_operation_selection_has_no_operation_metadata() {
    request_failure(
        Request::new("query A { ping } query B { ping }"),
        &["necrassrs.parse", "necrassrs.validate"],
        true,
    )
    .await;
}

#[tokio::test]
async fn unknown_operation_name_is_not_recorded_as_selected() {
    request_failure(
        Request::new("query A { ping }").with_operation_name("Missing"),
        &["necrassrs.parse", "necrassrs.validate"],
        true,
    )
    .await;
}

#[tokio::test]
async fn missing_variable_is_a_request_error_in_coercion() {
    request_failure(
        Request::new("query Input($input: String!) { echo(input: $input) }"),
        &[
            "necrassrs.parse",
            "necrassrs.validate",
            "necrassrs.coerce_variables",
        ],
        false,
    )
    .await;
}

#[tokio::test]
async fn custom_scalar_rejection_is_a_request_error_in_coercion() {
    request_failure(
        Request::new("query Input($input: Token!) { token(input: $input) }")
            .with_variables(serde_json::from_value(json!({"input": "reject-secret"})).unwrap()),
        &[
            "necrassrs.parse",
            "necrassrs.validate",
            "necrassrs.coerce_variables",
        ],
        false,
    )
    .await;
}

#[tokio::test]
async fn invalid_variable_value_is_a_request_error_in_coercion() {
    request_failure(
        Request::new("query Input($input: String!) { echo(input: $input) }")
            .with_variables(serde_json::from_value(json!({"input": ["invalid-secret"]})).unwrap()),
        &[
            "necrassrs.parse",
            "necrassrs.validate",
            "necrassrs.coerce_variables",
        ],
        false,
    )
    .await;
}

#[tokio::test]
async fn subscription_sent_to_single_response_execution_records_request_error() {
    let capture = Capture::default();
    let response = capture
        .run(execute(
            &schema(),
            &Request::new("subscription { scalar }"),
            &Query,
            &Context::default(),
        ))
        .await;
    assert!(response.requires_response_stream());
    let spans = capture.spans();
    let operation = one(&spans, "subscription");
    assert!(matches!(operation.status, Status::Error { .. }));
    assert_eq!(
        attr(operation, "error.type"),
        Some(&Value::from("necrassrs.request_error"))
    );
}

async fn check_phases(request: Request, probe: &str) {
    let capture = Capture::default();
    let response = capture
        .run(execute(&schema(), &request, &Query, &Context::default()))
        .await;
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({"data": {"token": 42}})
    );
    let spans = capture.spans();
    let operation = one(&spans, "query");
    let phases: Vec<_> = [
        "necrassrs.parse",
        "necrassrs.validate",
        "necrassrs.coerce_variables",
        "necrassrs.execute",
    ]
    .into_iter()
    .map(|name| one(&spans, name))
    .collect();
    for phase in &phases {
        child_of(phase, operation);
        assert_eq!(phase.span_kind, SpanKind::Internal);
    }
    for pair in phases.windows(2) {
        assert!(pair[0].end_time <= pair[1].start_time);
    }
    child_of(one(&spans, probe), phases[2]);
    child_of(one(&spans, "app.resolve"), phases[3]);
}

#[tokio::test]
async fn phases_are_ordered_and_include_custom_scalar_variable_validation() {
    check_phases(
        Request::new("query($input: Token!) { token(input: $input) }")
            .with_variables(serde_json::from_value(json!({"input": "accepted"})).unwrap()),
        "app.scalar.variable",
    )
    .await;
}

#[tokio::test]
async fn phases_include_custom_scalar_default_literal_validation() {
    check_phases(
        Request::new(r#"query($input: Token! = "accepted") { token(input: $input) }"#),
        "app.scalar.literal",
    )
    .await;
}

#[tokio::test]
async fn introspection_has_an_execution_phase_but_no_individual_field_spans() {
    let capture = Capture::default();
    let response = capture
        .run(execute(
            &schema(),
            &Request::new(
                "{ __schema { queryType { name } } __type(name: \"Node\") { name } __typename }",
            ),
            &Query,
            &Context::default(),
        ))
        .await;
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({"data": {"__schema": {"queryType": {"name": "Query"}}, "__type": {"name": "Node"}, "__typename": "Query"}})
    );
    let spans = capture.spans();
    child_of(one(&spans, "necrassrs.execute"), one(&spans, "query"));
    assert!(
        !spans
            .iter()
            .any(|span| attr(span, "necrassrs.graphql.field.path").is_some())
    );
}

#[tokio::test]
async fn disabled_introspection_is_reported_as_request_failure() {
    let capture = Capture::default();
    let options = ExecutionOptions {
        introspection: false,
    };
    let response = capture
        .run(execute_with_options(
            &schema(),
            &Request::new("{ __schema { queryType { name } } }"),
            &Query,
            &Context::default(),
            options,
        ))
        .await;
    assert!(response.is_request_error());
    let spans = capture.spans();
    assert_eq!(
        attr(one(&spans, "query"), "error.type"),
        Some(&Value::from("necrassrs.request_error"))
    );
}

#[tokio::test]
async fn recovered_application_error_does_not_fail_the_operation() {
    let capture = Capture::default();
    let response = capture
        .run(execute(
            &schema(),
            &Request::new("{ recovered }"),
            &Query,
            &Context::default(),
        ))
        .await;
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        json!({"data": {"recovered": 42}})
    );
    let spans = capture.spans();
    assert!(!one(&spans, "app.resolve").events.is_empty());
    let operation = one(&spans, "query");
    assert_eq!(operation.status, Status::Unset);
    assert_eq!(attr(operation, "error.type"), None);
}

#[tokio::test]
async fn concurrent_requests_keep_their_application_parent_context() {
    let capture = Capture::default();
    capture
        .run(async {
            let schema = schema();
            let context = Context::default();
            let first = Request::new("query First { node { child { value } } }");
            let second = Request::new("query Second { node { child { value } } }");
            let (a, b) = tokio::join!(
                execute(&schema, &first, &Query, &context)
                    .instrument(tracing::info_span!("http.first")),
                execute(&schema, &second, &Query, &context)
                    .instrument(tracing::info_span!("http.second")),
            );
            assert_eq!(
                serde_json::to_value(a).unwrap(),
                serde_json::to_value(b).unwrap()
            );
        })
        .await;
    let spans = capture.spans();
    for (name, parent) in [("First", "http.first"), ("Second", "http.second")] {
        let parent = one(&spans, parent);
        let operation = spans
            .iter()
            .find(|span| attr(span, "graphql.operation.name") == Some(&Value::from(name)))
            .unwrap();
        child_of(operation, parent);
        let trace: Vec<_> = spans
            .iter()
            .filter(|span| span.span_context.trace_id() == parent.span_context.trace_id())
            .cloned()
            .collect();
        child_of(one(&trace, "Query.node"), one(&trace, "necrassrs.execute"));
        child_of(one(&trace, "Node.child"), one(&trace, "Query.node"));
    }
    assert_ne!(
        one(&spans, "http.first").span_context.trace_id(),
        one(&spans, "http.second").span_context.trace_id()
    );
}

#[tokio::test]
async fn framework_spans_and_events_exclude_private_payloads() {
    let capture = Capture::default();
    let context = Context {
        secret: "context-secret".into(),
        ..Default::default()
    };
    let request = Request::new("query Privacy($input: String!) { echo(input: $input) fail bad literal: echo(input: \"literal-secret\") }")
        .with_variables(serde_json::from_value(json!({"input": "variable-secret"})).unwrap());
    let response = capture
        .run(execute(&schema(), &request, &Query, &context))
        .await;
    let response = serde_json::to_value(response).unwrap();
    assert_eq!(response["data"]["echo"], "response-secret");
    assert_eq!(response["errors"].as_array().unwrap().len(), 2);
    let spans = capture.spans();
    let framework: Vec<_> = spans
        .iter()
        .filter(|span| !span.name.starts_with("app."))
        .collect();
    assert!(
        framework.iter().any(|span| !span.events.is_empty()),
        "privacy must also be checked on actual framework error events"
    );
    let telemetry = format!("{framework:?}");
    for secret in [
        "query Privacy",
        "variable-secret",
        "literal-secret",
        "context-secret",
        "response-secret",
        "error-message-secret",
        "extension-secret",
        "invalid-int-secret",
    ] {
        assert!(
            !telemetry.contains(secret),
            "framework telemetry leaked {secret}"
        );
    }
    for span in framework {
        assert_eq!(attr(span, "graphql.document"), None);
    }
}

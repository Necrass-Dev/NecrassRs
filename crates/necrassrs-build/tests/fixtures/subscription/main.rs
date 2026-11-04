mod generated {
    include!(concat!(env!("OUT_DIR"), "/necrassrs.rs"));
}
mod resolvers;

fn main() {}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{StreamExt, executor::block_on};
    use necrassrs::{Request, Schema, subscribe};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn send<T: Send>(value: T) -> T {
        value
    }

    #[test]
    fn typed_inputs_and_borrowed_context_produce_event_specific_results() {
        let schema = Schema::parse_and_validate(generated::SDL, "embedded.graphql").unwrap();
        let dispatcher = generated::dispatch::SchemaDispatcher::new(
            resolvers::RootQuery,
            resolvers::RootMutation,
            resolvers::Feed,
        );
        let context = resolvers::AppContext {
            label: "Sheri".into(),
            creates: AtomicUsize::new(0),
        };
        let mut responses = {
            let request = Request::new("subscription($filter: Filter!, $token: Token!) { changed: orderChanged(filter: $filter, token: $token) { id status label } }")
                .with_variables(serde_json::from_value(json!({"filter": {"id": 42, "states": ["PAID", "SHIPPED"]}, "token": "allowed"})).unwrap());
            match block_on(send(subscribe(&schema, &request, &dispatcher, &context))) {
                Ok(stream) => send(stream),
                Err(response) => panic!("{}", serde_json::to_value(response).unwrap()),
            }
        };
        assert_eq!(context.creates.load(Ordering::SeqCst), 1);
        for status in ["PAID", "SHIPPED"] {
            let response = block_on(responses.next()).unwrap().unwrap();
            assert_eq!(
                serde_json::to_value(response).unwrap(),
                json!({"data": {"changed": {
                    "id": "42", "status": status, "label": "Sheri"
                }}})
            );
        }
        assert!(block_on(responses.next()).is_none());
        assert_eq!(context.creates.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn abstract_and_scalar_sources_and_existing_roots_share_one_dispatcher() {
        let schema = Schema::parse_and_validate(generated::SDL, "embedded.graphql").unwrap();
        let dispatcher = generated::dispatch::SchemaDispatcher::new(
            resolvers::RootQuery,
            resolvers::RootMutation,
            resolvers::Feed,
        );
        let context = resolvers::AppContext {
            label: "Sheri".into(),
            creates: AtomicUsize::new(0),
        };
        for (document, expected) in [
            (
                "subscription { updates { __typename ... on Order { id } ... on Notice { text } } }",
                vec![
                    json!({"data": {"updates": {"__typename": "Order", "id": "42"}}}),
                    json!({"data": {"updates": {"__typename": "Notice", "text": "Delivered"}}}),
                ],
            ),
            (
                "subscription { count }",
                vec![json!({"data": {"count": 1}}), json!({"data": {"count": 2}})],
            ),
        ] {
            let request = Request::new(document);
            let mut responses = match block_on(subscribe(&schema, &request, &dispatcher, &context))
            {
                Ok(stream) => stream,
                Err(_) => panic!("valid subscription rejected"),
            };
            for expected in expected {
                assert_eq!(
                    serde_json::to_value(block_on(responses.next()).unwrap().unwrap()).unwrap(),
                    expected
                );
            }
            assert!(block_on(responses.next()).is_none());
        }
        for (document, expected) in [
            ("{ ping }", json!({"data": {"ping": 1}})),
            ("mutation { update }", json!({"data": {"update": 2}})),
        ] {
            let response = block_on(necrassrs::execute(
                &schema,
                &Request::new(document),
                &dispatcher,
                &context,
            ));
            assert_eq!(serde_json::to_value(response).unwrap(), expected);
        }
    }

    #[test]
    fn scalar_literal_kinds_and_invalid_variables_are_checked_before_source_creation() {
        let schema = Schema::parse_and_validate(generated::SDL, "embedded.graphql").unwrap();
        let dispatcher = generated::dispatch::SchemaDispatcher::new(
            resolvers::RootQuery,
            resolvers::RootMutation,
            resolvers::Feed,
        );
        let context = resolvers::AppContext {
            label: "Sheri".into(),
            creates: AtomicUsize::new(0),
        };
        for token in ["allowed", "123", "\"denied\""] {
            let request = Request::new(format!(
                "subscription {{ orderChanged(filter: {{id: 42, states: PAID}}, token: {token}) {{ id }} }}"
            ));
            match block_on(subscribe(&schema, &request, &dispatcher, &context)) {
                Err(response) => assert!(response.is_request_error()),
                Ok(_) => panic!("invalid scalar literal accepted"),
            }
        }
        let request = Request::new("subscription($token: Token!) { orderChanged(filter: {id: 42, states: PAID}, token: $token) { id } }")
            .with_variables(serde_json::from_value(json!({"token": "denied"})).unwrap());
        match block_on(subscribe(&schema, &request, &dispatcher, &context)) {
            Err(response) => assert!(response.is_request_error()),
            Ok(_) => panic!("invalid scalar variable accepted"),
        }
        assert_eq!(context.creates.load(Ordering::SeqCst), 0);
        let request = Request::new(
            "subscription { orderChanged(filter: {id: 42, states: PAID, note: null}, token: \"allowed\") { id status } }",
        );
        let mut responses = match block_on(subscribe(&schema, &request, &dispatcher, &context)) {
            Ok(stream) => stream,
            Err(_) => panic!("valid string literal rejected"),
        };
        assert_eq!(
            serde_json::to_value(block_on(responses.next()).unwrap().unwrap()).unwrap(),
            json!({"data": {"orderChanged": {"id": "42", "status": "PAID"}}})
        );
        assert_eq!(context.creates.load(Ordering::SeqCst), 1);
    }
}

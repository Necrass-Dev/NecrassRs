mod generated {
    include!(concat!(env!("OUT_DIR"), "/necrassrs.rs"));
}

mod resolvers;

fn main() {
    let schema = necrassrs::Schema::parse_and_validate(generated::SDL, "embedded.graphql").unwrap();
    let dispatcher = generated::dispatch::SchemaDispatcher::new(resolvers::Query);
    futures::executor::block_on(async {
        for document in std::env::args().skip(1) {
            let request = necrassrs::Request::new(document);
            let response = necrassrs::execute(&schema, &request, &dispatcher, &()).await;
            println!("{}", serde_json::to_string(&response).unwrap());
        }
    });
}

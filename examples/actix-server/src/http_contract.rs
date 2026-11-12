use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

fn exchange(
    address: SocketAddr,
    method: &str,
    target: &str,
    accept: &str,
    body: &str,
) -> (u16, String, String) {
    let timeout = Duration::from_secs(5);
    let mut stream = TcpStream::connect_timeout(&address, timeout).unwrap();
    stream.set_read_timeout(Some(timeout)).unwrap();
    stream.set_write_timeout(Some(timeout)).unwrap();
    write!(
        stream,
        "{method} {target} HTTP/1.1\r\nHost: {address}\r\nAccept: {accept}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
    ).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").expect("HTTP response head");
    let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, headers.to_ascii_lowercase(), body.to_owned())
}

// The test server uses an application-selected 128-byte body limit.
pub fn check_method_and_transport_errors(address: SocketAddr) {
    let query = r#"{"query":"{ hello(name: \"Sheri\") }"}"#;
    for (method, target, body) in [
        ("GET", "/graphql?query=%7Bhello(name%3A%22Sheri%22)%7D", ""),
        ("POST", "/graphql", query),
    ] {
        let (status, headers, body) = exchange(
            address,
            method,
            target,
            "application/json;q=0.5\r\nAccept: application/graphql-response+json",
            body,
        );
        assert_eq!(status, 200);
        assert!(headers.contains("\r\ncontent-type: application/graphql-response+json"));
        assert!(headers.contains("\r\nvary: accept"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({ "data": { "hello": "Hello, Sheri" } })
        );
    }

    let oversized = " ".repeat(129);
    for (body, expected) in [("{", 400), ("{}", 422), (oversized.as_str(), 413)] {
        let (status, _, body) = exchange(address, "POST", "/graphql", "text/event-stream", body);
        assert_eq!(status, expected);
        assert!(body.is_empty());
    }

    let mut observed = Vec::new();
    for (method, target, accept) in [
        ("HEAD", "/graphql", "text/html"),
        (
            "GET",
            "/graphql?query=mutation%7Bhello(name%3A%22Sheri%22)%7D",
            "text/event-stream",
        ),
    ] {
        let (status, headers, body) = exchange(address, method, target, accept, "");
        let allows_post = headers.lines().any(|line| {
            line.strip_prefix("allow:")
                .is_some_and(|value| value.split(',').any(|method| method.trim() == "post"))
        });
        observed.push((status, allows_post, body.is_empty()));
    }

    // Exceed the locked http 0.2/1.x URI representation limit, not a project-defined URI limit.
    let oversized_uri = format!(
        "/graphql?query=%7B__typename%7D&ignored={}",
        "a".repeat(u16::MAX as usize)
    );
    let (uri_status, uri_headers, _) = exchange(
        address,
        "GET",
        &oversized_uri,
        "application/graphql-response+json",
        "",
    );
    assert!(!uri_headers.contains("application/graphql-response+json"));

    // A rejected request must not prevent the next request from executing.
    let (status, _, body) = exchange(address, "POST", "/graphql", "application/json", query);
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({ "data": { "hello": "Hello, Sheri" } })
    );
    assert_eq!(
        (observed, uri_status),
        (vec![(405, true, true); 2], 400),
        "HEAD, SSE-only GET mutation, and oversized URI (HTTP candidate 3903e680, Status Codes)"
    );
}

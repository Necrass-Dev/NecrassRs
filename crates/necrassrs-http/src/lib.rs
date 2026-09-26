pub const GRAPHQL_JSON: &str = "application/graphql-response+json";
pub const JSON: &str = "application/json";

pub fn response_media_type<'a>(headers: impl IntoIterator<Item = &'a str>) -> Option<&'static str> {
    let mut present = false;
    let mut scores = [None, None];
    for header in headers {
        present = true;
        let mut quoted = false;
        let mut escaped = false;
        let ranges = header.split(|character| {
            if escaped {
                escaped = false;
            } else if quoted && character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = !quoted;
            }
            character == ',' && !quoted
        });
        for range in ranges.filter(|range| !range.trim().is_empty()) {
            let range = range.trim().parse::<mime::Mime>().ok()?;
            let mut quality = 1000;
            let mut has_quality = false;
            let mut parameters = 0;
            let mut supported_parameters = true;
            for (name, value) in range.params() {
                if name == "q" {
                    if has_quality {
                        return None;
                    }
                    quality = parse_quality(value.as_str())?;
                    has_quality = true;
                } else if name == mime::CHARSET && value.as_str().eq_ignore_ascii_case("utf-8") {
                    parameters += 1;
                } else {
                    supported_parameters = false;
                }
            }
            if !supported_parameters {
                continue;
            }
            for (index, candidate) in [GRAPHQL_JSON, JSON].iter().enumerate() {
                let specificity = if range.essence_str() == *candidate {
                    2
                } else if range.essence_str() == "application/*" {
                    1
                } else if range.essence_str() == "*/*" {
                    0
                } else {
                    continue;
                };
                let score = (specificity, parameters, quality);
                if scores[index].is_none_or(|previous| score > previous) {
                    scores[index] = Some(score);
                }
            }
        }
    }
    if !present {
        return Some(JSON);
    }
    let graphql = scores[0].map_or(0, |(_, _, quality)| quality);
    let json = scores[1].map_or(0, |(_, _, quality)| quality);
    match (graphql, json) {
        (0, 0) => None,
        _ if graphql >= json => Some(GRAPHQL_JSON),
        _ => Some(JSON),
    }
}

fn parse_quality(value: &str) -> Option<u16> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 3 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    match whole {
        "0" => Some(fraction.parse::<u16>().unwrap_or(0) * 10_u16.pow(3 - fraction.len() as u32)),
        "1" if fraction.bytes().all(|byte| byte == b'0') => Some(1000),
        _ => None,
    }
}

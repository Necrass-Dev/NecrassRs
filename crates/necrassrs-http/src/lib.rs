//! Framework-independent response negotiation shared by the HTTP adapters.

/// The GraphQL response media type.
pub const GRAPHQL_JSON: &str = "application/graphql-response+json";
/// The legacy JSON response media type.
pub const JSON: &str = "application/json";

/// Selects a supported response media type from all `Accept` header values.
///
/// Specific ranges override wildcards, including explicit `q=0` exclusions.
/// Equal quality prefers GraphQL JSON. Missing headers preserve legacy JSON.
/// Invalid headers and requests accepting neither type return `None`.
pub fn response_media_type<'a>(headers: impl IntoIterator<Item = &'a str>) -> Option<&'static str> {
    let mut present = false;
    let mut scores = [None, None];
    for header in headers {
        present = true;
        let ranges = split_unquoted(header, ',');
        for range in ranges.filter(|range| !range.trim().is_empty()) {
            // MIME parsing rejects HTTP optional whitespace around semicolons.
            let normalized = split_unquoted(range, ';')
                .map(|part| part.trim_matches([' ', '\t']))
                .collect::<Vec<_>>()
                .join(";");
            let range = normalized.parse::<mime::Mime>().ok()?;
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
            for (current, score) in scores
                .iter_mut()
                .zip(media_range_scores(&range, parameters, quality))
            {
                *current = (*current).max(score);
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

fn media_range_scores(
    range: &mime::Mime,
    parameters: usize,
    quality: u16,
) -> [Option<(u8, usize, u16)>; 2] {
    [GRAPHQL_JSON, JSON].map(|candidate| {
        let specificity = match range.essence_str() {
            exact if exact == candidate => 2,
            "application/*" => 1,
            "*/*" => 0,
            _ => return None,
        };
        Some((specificity, parameters, quality))
    })
}

fn split_unquoted(value: &str, separator: char) -> impl Iterator<Item = &str> {
    let mut quoted = false;
    let mut escaped = false;
    value.split(move |character| {
        if escaped {
            escaped = false;
        } else if quoted && character == '\\' {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        }
        character == separator && !quoted
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_optional_whitespace_around_parameter_separators() {
        let headers = [
            "application/json ; q=1",
            "application/json;\tq=1",
            "application/json;charset=utf-8 ;q=1",
        ];
        assert_eq!(
            headers.map(|accept| response_media_type([accept])),
            [Some(JSON); 3],
        );
    }

    #[test]
    fn negotiates_quality_specificity_and_exclusions() {
        assert_eq!(response_media_type([]), Some(JSON));
        for (accept, expected) in [
            (GRAPHQL_JSON, Some(GRAPHQL_JSON)),
            (JSON, Some(JSON)),
            ("*/*", Some(GRAPHQL_JSON)),
            ("application/*", Some(GRAPHQL_JSON)),
            (
                "application/json;q=1, application/graphql-response+json;q=0.5",
                Some(JSON),
            ),
            (
                "application/json;q=0.5, application/graphql-response+json;q=1",
                Some(GRAPHQL_JSON),
            ),
            ("application/graphql-response+json;q=0, */*;q=1", Some(JSON)),
            (
                "application/json;q=0, application/graphql-response+json;q=0, */*",
                None,
            ),
            ("application/json;charset=utf-8", Some(JSON)),
            ("application/json;charset=ascii", None),
            ("application/json ; charset=\"utf-8\" ; q=1", Some(JSON)),
            ("application/json;charset=\" utf-8 \"", None),
            ("application/json;charset=\"utf-8 ;\"", None),
            ("application /json;q=1", None),
            ("application/json;q =1", None),
            ("application/json;q= 1", None),
            ("text/html;example=\"a,b\", application/json", Some(JSON)),
            ("text/html", None),
            ("", None),
            ("application/json;q=NaN", None),
            ("application/json;q=1.001", None),
            ("application/json;q=0.0001", None),
            ("application/json;q=-1", None),
        ] {
            assert_eq!(response_media_type([accept]), expected, "{accept}");
        }
        assert_eq!(
            response_media_type(["application/json;q=0.5", GRAPHQL_JSON]),
            Some(GRAPHQL_JSON)
        );
    }
}

use serde_json::{Map, Value, json};

pub const DELAY_MILLISECONDS: u64 = 2;
pub const DEEP_PATH_DEPTH: usize = 8;
pub const FAN_OUTS: [usize; 3] = [1, 4, 16];

pub const SDL: &str = r#"
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
"#;

#[derive(Clone, Copy)]
pub enum Workload {
    ImmediateRoot,
    DelayedRoot,
    DelayedNested,
    DelayedList,
    DelayedDeepPath,
}

impl Workload {
    pub const ALL: [Self; 5] = [
        Self::ImmediateRoot,
        Self::DelayedRoot,
        Self::DelayedNested,
        Self::DelayedList,
        Self::DelayedDeepPath,
    ];

    pub fn group_name(self) -> &'static str {
        match self {
            Self::ImmediateRoot => "immediate_root_fields",
            Self::DelayedRoot => "delayed_root_fields",
            Self::DelayedNested => "delayed_nested_siblings",
            Self::DelayedList => "delayed_list_items",
            Self::DelayedDeepPath => "delayed_deep_path",
        }
    }

    pub fn sizes(self) -> &'static [usize] {
        match self {
            Self::DelayedDeepPath => &[DEEP_PATH_DEPTH],
            _ => &FAN_OUTS,
        }
    }

    pub fn size_label(self, size: usize) -> String {
        match self {
            Self::DelayedDeepPath => format!("depth_{size}"),
            _ => format!("fan_out_{size}"),
        }
    }
}

pub struct Case {
    pub query: String,
    pub expected_data: Value,
}

impl Case {
    pub fn new(workload: Workload, size: usize) -> Self {
        match workload {
            Workload::ImmediateRoot => root_case("immediate", size),
            Workload::DelayedRoot => root_case("delayed", size),
            Workload::DelayedNested => nested_case(size),
            Workload::DelayedList => list_case(size),
            Workload::DelayedDeepPath => deep_path_case(size),
        }
    }
}

fn selected_fields(field: &str, fan_out: usize) -> String {
    (0..fan_out)
        .map(|index| format!("f{index}: {field}(value: {index})"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn field_values(fan_out: usize) -> Value {
    Value::Object(
        (0..fan_out)
            .map(|index| (format!("f{index}"), json!(index)))
            .collect::<Map<_, _>>(),
    )
}

fn root_case(field: &str, fan_out: usize) -> Case {
    Case {
        query: format!("{{ {} }}", selected_fields(field, fan_out)),
        expected_data: field_values(fan_out),
    }
}

fn nested_case(fan_out: usize) -> Case {
    Case {
        query: format!("{{ nested {{ {} }} }}", selected_fields("delayed", fan_out)),
        expected_data: json!({ "nested": field_values(fan_out) }),
    }
}

fn list_case(fan_out: usize) -> Case {
    Case {
        query: format!("{{ items(count: {fan_out}) {{ value }} }}"),
        expected_data: json!({
            "items": (0..fan_out)
                .map(|index| json!({ "value": index }))
                .collect::<Vec<_>>()
        }),
    }
}

fn deep_path_case(depth: usize) -> Case {
    let mut selection = String::from("value");
    let mut expected = json!({ "value": 1 });
    for _ in 0..depth {
        selection = format!("next {{ {selection} }}");
        expected = json!({ "next": expected });
    }

    Case {
        query: format!("{{ chain {{ {selection} }} }}"),
        expected_data: json!({ "chain": expected }),
    }
}

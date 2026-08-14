use au::api::{parse_read, Plan};
use serde_json::Value;

fn next(state: &mut u64) -> u8 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state as u8
}

#[test]
fn bounded_random_agent_inputs_never_panic() {
    let mut state = 0x9e3779b97f4a7c15;
    for case in 0..10_000 {
        let len = (next(&mut state) as usize * 4 + case % 17).min(1024);
        let mut bytes = vec![0; len];
        for byte in &mut bytes {
            *byte = next(&mut state);
        }
        let run = std::panic::catch_unwind(|| {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                let _ = Plan::parse(value.clone());
                let _ = parse_read(value);
            }
        });
        assert!(run.is_ok(), "parser panicked on case {case}");
    }
}

#[test]
fn mutated_golden_plan_never_bypasses_limits() {
    let golden: Value = serde_json::from_str(include_str!("../../protocol-golden.json")).unwrap();
    for count in 0..80 {
        let mut plan = golden["plan"].clone();
        plan["p"] = Value::Array((0..count).map(|_| serde_json::json!(["assert", ["text", "x"]])).collect());
        let result = Plan::parse(plan);
        assert_eq!(result.is_ok(), (1..=32).contains(&count));
    }
}

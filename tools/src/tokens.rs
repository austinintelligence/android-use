//! Token budgets for what an agent reads. Counts are bytes / 4, the estimate the documentation
//! budgets use; `bench.py` in docs/design/baseline measures real tokenizer counts.
//!
//! * fixed: the server instructions and both tool definitions, paid on every turn;
//! * screens: `screen` rendered from golden fixtures, captured on a device with
//!   `cargo xtask golden-capture NAME --reviewed`;
//! * the installed skill bundle is gated with the documentation budgets.
use super::{approx_tokens, ioe, walk};
use serde_json::Value;
use std::{fs, path::Path};

/// Instructions plus tool definitions, in tokens.
const FIXED_MAX: usize = 420;
/// All golden screens rendered, in tokens.
const SCREENS_MAX: usize = 360;

pub fn fixed_tokens() -> Result<usize, String> {
    let tools = serde_json::to_vec(&au::api::tool_schemas()).map_err(ioe)?;
    Ok(approx_tokens(au::spec::INSTRUCTIONS.len() + tools.len()))
}

pub fn screen_tokens(root: &Path) -> Result<(usize, usize), String> {
    let dir = root.join("computer/tests/fixtures/screens");
    let (mut bytes, mut count) = (0, 0);
    for path in walk(&dir)? {
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let fixture: Value = serde_json::from_slice(&fs::read(&path).map_err(ioe)?).map_err(ioe)?;
        let text = au::render_screen(&fixture["rows"], false).map_err(|e| format!("{}: {}", path.display(), e.message))?;
        if let Some(dump) = std::env::var_os("TOKENS_DUMP") {
            fs::create_dir_all(&dump).map_err(ioe)?;
            fs::write(Path::new(&dump).join(path.file_stem().unwrap_or_default()).with_extension("txt"), &text).map_err(ioe)?;
        }
        bytes += text.len();
        count += 1;
    }
    Ok((approx_tokens(bytes), count))
}

pub fn gate(root: &Path) -> Result<(), String> {
    let fixed = fixed_tokens()?;
    let (screens, count) = screen_tokens(root)?;
    println!("token_budget fixed_tokens={fixed} max={FIXED_MAX} screens_tokens={screens} screens={count} max={SCREENS_MAX}");
    if fixed > FIXED_MAX {
        return Err(format!("fixed per-turn tokens {fixed} exceed {FIXED_MAX}"));
    }
    if screens > SCREENS_MAX {
        return Err(format!("golden screens render to {screens} tokens, over {SCREENS_MAX}"));
    }
    Ok(())
}

/// Saves the rows of the screen now in front on the enrolled device as a golden fixture. Fixtures
/// are committed, so the person capturing confirms with `--reviewed` that the screen shows nothing
/// personal, and rows that still look personal (an email, a phone or card number) are refused.
pub fn capture(root: &Path, name: &str, reviewed: bool) -> Result<(), String> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err("golden name must be letters, digits, or underscores".into());
    }
    if !reviewed {
        return Err("golden fixtures are committed: check the screen on the device shows no names, accounts, messages, or numbers of yours, then pass --reviewed".into());
    }
    let paths = au::device::Paths::discover().map_err(|e| e.message)?;
    let mut bridge = au::link::DeviceLink::helper(&mut au::link::AdbLink::new(paths)).map_err(|e| e.message)?;
    let rows = bridge.query("semantic", Value::String(String::new())).map_err(|e| e.message)?;
    if let Some(found) = personal_data(&rows) {
        return Err(format!("the screen shows what looks like personal data ({found}); nothing was saved"));
    }
    let dir = root.join("computer/tests/fixtures/screens");
    fs::create_dir_all(&dir).map_err(ioe)?;
    let path = dir.join(format!("{name}.json"));
    fs::write(&path, serde_json::to_vec_pretty(&serde_json::json!({"rows": rows})).map_err(ioe)?).map_err(ioe)?;
    println!("saved {}", path.display());
    Ok(())
}

/// The kind of the first string in `value` that looks like personal data: an email address, a card
/// number (13 to 19 digits passing the Luhn check), or a phone number (10 to 15 digits, possibly
/// grouped by spaces, dashes or brackets; dates and times do not count). A backstop to `--reviewed`,
/// not a substitute: names and messages cannot be recognized.
pub fn personal_data(value: &Value) -> Option<&'static str> {
    match value {
        Value::String(text) => personal_text(text),
        Value::Array(items) => items.iter().find_map(personal_data),
        Value::Object(map) => map.values().find_map(personal_data),
        _ => None,
    }
}

fn personal_text(text: &str) -> Option<&'static str> {
    let email = text.split(|c: char| c.is_whitespace() || "<>()[]\",;:".contains(c)).any(|word| {
        let Some((local, domain)) = word.trim_end_matches(['.', '!', '?']).split_once('@') else { return false };
        !local.is_empty() && domain.split('.').count() >= 2 && domain.split('.').all(|part| !part.is_empty() && part.chars().all(|c| c.is_alphanumeric() || c == '-'))
    });
    if email {
        return Some("an email address");
    }
    let mut chars: Vec<char> = text.chars().collect();
    // Dates and times sit next to other numbers on ordinary screens; blank them so they never
    // join a digit run. Only standalone ones: `4111-11-11-1111...` is a card, not a date.
    let joined = |c: Option<&char>| c.is_some_and(|c| c.is_ascii_digit() || "-/.".contains(*c));
    for at in 0..chars.len() {
        let shape = ["dddd-dd-dd", "dd-dd-dddd", "dd:dd", "d:dd"].into_iter().find(|shape| shaped(&chars[at..], shape));
        if let Some(len) = shape.map(str::len).filter(|&len| !joined(at.checked_sub(1).and_then(|i| chars.get(i))) && !joined(chars.get(at + len))) {
            chars[at..at + len].fill('x');
        }
    }
    let mut at = 0;
    while at < chars.len() {
        if !(chars[at].is_ascii_digit() || chars[at] == '+' || chars[at] == '(') {
            at += 1;
            continue;
        }
        let start = at;
        while at < chars.len() && (chars[at].is_ascii_digit() || " -()+".contains(chars[at])) {
            at += 1;
        }
        let run: String = chars[start..at].iter().collect();
        let digits: Vec<u32> = run.chars().filter_map(|c| c.to_digit(10)).collect();
        if (13..=19).contains(&digits.len()) && luhn(&digits) {
            return Some("a card number");
        }
        if (10..=15).contains(&digits.len()) {
            return Some("a phone number");
        }
    }
    None
}

/// `chars` starts with `shape`: `d` is a digit, `-` a date separator (`-`, `/` or `.`), anything
/// else itself.
fn shaped(chars: &[char], shape: &str) -> bool {
    chars.len() >= shape.len()
        && shape.chars().zip(chars).all(|(want, &c)| match want {
            'd' => c.is_ascii_digit(),
            '-' => "-/.".contains(c),
            _ => c == want,
        })
}

fn luhn(digits: &[u32]) -> bool {
    const DOUBLED: [u32; 10] = [0, 2, 4, 6, 8, 1, 3, 5, 7, 9];
    let sum: u32 = digits.iter().rev().enumerate().map(|(i, &d)| if i % 2 == 1 { DOUBLED[d as usize] } else { d }).sum();
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    use super::personal_data;
    use serde_json::json;

    #[test]
    fn captures_refuse_rows_that_look_personal() {
        assert_eq!(personal_data(&json!([["Signed in as ada@example.com", "", "text"]])), Some("an email address"));
        assert_eq!(personal_data(&json!([["Sent to ada@example.com.", "", "text"]])), Some("an email address"));
        assert_eq!(personal_data(&json!([["Call +1 (650) 555-0134", "", "button"]])), Some("a phone number"));
        for card in ["4111 1111 1111 1111", "4111-1111-1111-1111", "4111111111111111", "4111-11-11-11111111"] {
            assert_eq!(personal_data(&json!({"rows": [[card]]})), Some("a card number"), "{card}");
        }
        assert_eq!(personal_data(&json!([["2026-10-15 650 555 0134"]])), Some("a phone number"));
        let ordinary = json!([
            ["Wi-Fi", "on", "switch"],
            ["Updated 2026-10-15 12:30", "", "text"],
            ["(2026-10-15 12:30)", "", "text"],
            ["12:30 2026-10-15", "", "text"],
            ["15-10-2026 12:30", "", "text"],
            ["2026-10-15 - 2026-10-17", "", "text"],
            ["Storage 64.0 GB", "", "text"],
            ["Version 1.2.0 (1002000)", "", "text"],
            ["Build 1.2.0.1003000", "", "text"],
            ["@android", "", "text"]
        ]);
        assert_eq!(personal_data(&ordinary), None);
    }
}

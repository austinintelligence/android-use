//! Token budgets for what an agent reads. Counts are bytes / 4, the estimate the documentation
//! budgets use; `bench.py` in docs/design/baseline measures real tokenizer counts.
//!
//! * fixed: the server instructions and both tool definitions, paid on every turn;
//! * screens: `screen` rendered from golden fixtures, captured on a device with
//!   `cargo xtask golden-capture NAME`;
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

/// Saves the rows of the screen now in front on the enrolled device as a golden fixture.
pub fn capture(root: &Path, name: &str) -> Result<(), String> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err("golden name must be letters, digits, or underscores".into());
    }
    let paths = au::device::Paths::discover().map_err(|e| e.message)?;
    let adb = au::device::Adb::discover().map_err(|e| e.message)?;
    let device = adb.resolve(&paths).map_err(|e| e.message)?;
    let mut bridge = au::bridge::Bridge::connect(adb, device).map_err(|e| e.message)?;
    let rows = bridge.query("semantic", Value::String(String::new())).map_err(|e| e.message)?;
    let dir = root.join("computer/tests/fixtures/screens");
    fs::create_dir_all(&dir).map_err(ioe)?;
    let path = dir.join(format!("{name}.json"));
    fs::write(&path, serde_json::to_vec_pretty(&serde_json::json!({"rows": rows})).map_err(ioe)?).map_err(ioe)?;
    println!("saved {}", path.display());
    Ok(())
}

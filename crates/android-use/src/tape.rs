use std::collections::HashMap;

use serde::Serialize;

use crate::batch;
use crate::error::{AuError, Result};
use crate::selector::Selector;

pub const TAPE_VERSION: u8 = 1;
pub const MAX_DICTIONARY_ENTRIES: usize = 32;
pub const MAX_DICTIONARY_VALUE_BYTES: usize = 8 * 1024;
pub const OPCODES: &[char] = &[
    'D', 'R', 'F', 'T', 'L', 'E', 'S', 'W', 'A', 'P', 'K', 'H', 'B', 'G', 'Q', 'Y',
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Op {
    Dict {
        slot: u8,
        value: String,
    },
    Reset,
    Find {
        slot: u8,
        selector: String,
    },
    Tap {
        target: String,
    },
    Long {
        target: String,
    },
    Set {
        target: String,
        text: String,
    },
    Scroll {
        target: String,
        direction: String,
    },
    Wait {
        selector: String,
        timeout_ms: u64,
    },
    Assert {
        selector: String,
        timeout_ms: u64,
    },
    Proof {
        selector: String,
        postcondition: String,
        timeout_ms: u64,
    },
    Key {
        key: String,
    },
    Home,
    Back,
    TapAt {
        x: String,
        y: String,
    },
    Frontier,
    /// Parser-only bounded repeat. `parse` expands this before returning a
    /// program, so the execution engine never contains an unbounded loop.
    Repeat {
        count: u8,
        op: Box<Op>,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Program {
    pub ops: Vec<Op>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Disassembly {
    pub version: u8,
    pub expanded: bool,
    pub instructions: usize,
    pub state_actions: usize,
    pub lines: Vec<String>,
}

/// Values defined by D0..D31 live for the lifetime of the daemon session.
/// Handles are deliberately not stored here: they belong to one tape run and
/// are invalidated with the helper's scene generation.
#[derive(Clone, Debug)]
pub struct TapeSession {
    values: HashMap<u8, String>,
    pub epoch: u64,
}

impl Default for TapeSession {
    fn default() -> Self {
        Self {
            values: HashMap::new(),
            epoch: 1,
        }
    }
}

impl TapeSession {
    pub fn define(&mut self, slot: u8, value: String) -> Result<()> {
        validate_slot(slot)?;
        if value.len() > MAX_DICTIONARY_VALUE_BYTES {
            return Err(AuError::code(
                "E_TAPE",
                format!("dictionary value exceeds {MAX_DICTIONARY_VALUE_BYTES} bytes"),
            ));
        }
        if !self.values.contains_key(&slot) && self.values.len() >= MAX_DICTIONARY_ENTRIES {
            return Err(AuError::code(
                "E_TAPE",
                format!("dictionary may not exceed {MAX_DICTIONARY_ENTRIES} entries"),
            ));
        }
        self.values.insert(slot, value);
        self.epoch = self.epoch.wrapping_add(1).max(1);
        Ok(())
    }

    pub fn reset(&mut self) {
        self.values.clear();
        self.epoch = self.epoch.wrapping_add(1).max(1);
    }

    pub fn resolve(&self, value: &str) -> Result<String> {
        let Some(slot) = value.strip_prefix('@') else {
            return Ok(value.into());
        };
        let slot = parse_slot(slot)?;
        self.values
            .get(&slot)
            .cloned()
            .ok_or_else(|| AuError::code("E_DICT", format!("dictionary slot @{slot} is undefined")))
    }

    pub fn checksum(&self) -> String {
        let mut hash = 2_166_136_261u32;
        for slot in 0..MAX_DICTIONARY_ENTRIES as u8 {
            if let Some(value) = self.values.get(&slot) {
                hash ^= u32::from(slot);
                hash = hash.wrapping_mul(16_777_619);
                for byte in value.as_bytes() {
                    hash ^= u32::from(*byte);
                    hash = hash.wrapping_mul(16_777_619);
                }
            }
        }
        format!("{hash:08x}")
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

pub fn parse(input: &str) -> Result<Program> {
    let rows = batch::tokenize_statements(input)?;
    if rows.is_empty() || rows.iter().all(Vec::is_empty) {
        return Err(AuError::code("E_TAPE", "tape is empty"));
    }
    if rows.len() > batch::MAX_INSTRUCTIONS {
        return Err(AuError::code(
            "E_TAPE",
            format!(
                "tape may not exceed {} instructions",
                batch::MAX_INSTRUCTIONS
            ),
        ));
    }
    let mut ops = Vec::with_capacity(rows.len());
    for tokens in rows {
        if tokens.is_empty() {
            continue;
        }
        let op = parse_op(&tokens)?;
        expand(op, &mut ops)?;
    }
    let state_actions = ops.iter().filter(|op| is_state_changing(op)).count();
    if state_actions > batch::MAX_STATE_ACTIONS {
        return Err(AuError::code(
            "E_TAPE",
            format!(
                "tape may not execute more than {} state-changing actions",
                batch::MAX_STATE_ACTIONS
            ),
        ));
    }
    if ops.is_empty() {
        return Err(AuError::code("E_TAPE", "tape is empty"));
    }
    Ok(Program { ops })
}

/// Decode a model tape for human diagnostics without resolving a device,
/// opening a helper session, or executing any instruction. The parser is the
/// same parser used by execution; repeat syntax is shown in its bounded,
/// expanded form so the diagnostic proves the actual instruction and state
/// limits that will be applied.
pub fn disassemble(input: &str) -> Result<Disassembly> {
    let program = parse(input)?;
    let state_actions = program
        .ops
        .iter()
        .filter(|op| is_state_changing(op))
        .count();
    let mut lines = Vec::with_capacity(program.ops.len());
    let mut bytes = 0usize;
    for (index, op) in program.ops.iter().enumerate() {
        let line = format!("{index:02} {}", format_op(op));
        bytes = bytes
            .checked_add(line.len() + 1)
            .ok_or_else(|| AuError::code("E_OUTPUT_LIMIT", "disassembly size overflow"))?;
        if bytes > crate::MAX_OUTPUT_BYTES {
            return Err(AuError::code(
                "E_OUTPUT_LIMIT",
                format!("disassembly exceeds {} bytes", crate::MAX_OUTPUT_BYTES),
            ));
        }
        lines.push(line);
    }
    Ok(Disassembly {
        version: TAPE_VERSION,
        expanded: true,
        instructions: program.ops.len(),
        state_actions,
        lines,
    })
}

fn format_op(op: &Op) -> String {
    match op {
        Op::Dict { slot, value } => format!("D{slot} {}", quote(value)),
        Op::Reset => "R".into(),
        Op::Find { slot, selector } => format!("F{slot} {}", quote(selector)),
        Op::Tap { target } => format!("T {}", quote(target)),
        Op::Long { target } => format!("L {}", quote(target)),
        Op::Set { target, text } => format!("E {} {}", quote(target), quote(text)),
        Op::Scroll { target, direction } => format!("S {} {direction}", quote(target)),
        Op::Wait {
            selector,
            timeout_ms,
        } => format!("W {} {timeout_ms}", quote(selector)),
        Op::Assert {
            selector,
            timeout_ms,
        } => format!("A {} {timeout_ms}", quote(selector)),
        Op::Proof {
            selector,
            postcondition,
            timeout_ms,
        } => format!(
            "P {} {} {timeout_ms}",
            quote(selector),
            quote(postcondition)
        ),
        Op::Key { key } => format!("K {}", quote(key)),
        Op::Home => "H".into(),
        Op::Back => "B".into(),
        Op::TapAt { x, y } => format!("G {x} {y}"),
        Op::Frontier => "Q".into(),
        Op::Repeat { count, op } => format!("Y{count} {}", format_op(op)),
    }
}

fn quote(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 2);
    result.push('\'');
    for character in value.chars() {
        match character {
            '\\' => result.push_str("\\\\"),
            '\'' => result.push_str("\\'"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            character if character.is_control() => {
                result.push_str(&format!("\\u{{{:x}}}", character as u32));
            }
            character => result.push(character),
        }
    }
    result.push('\'');
    result
}

fn expand(op: Op, output: &mut Vec<Op>) -> Result<()> {
    match op {
        Op::Repeat { count, op } => {
            for _ in 0..count {
                expand((*op).clone(), output)?;
            }
        }
        op => {
            if output.len() >= batch::MAX_INSTRUCTIONS {
                return Err(AuError::code(
                    "E_TAPE",
                    format!(
                        "tape may not exceed {} instructions",
                        batch::MAX_INSTRUCTIONS
                    ),
                ));
            }
            output.push(op);
        }
    }
    Ok(())
}

fn parse_op(tokens: &[String]) -> Result<Op> {
    let raw = tokens
        .first()
        .ok_or_else(|| AuError::code("E_TAPE", "missing opcode"))?;
    let opcode = raw
        .chars()
        .next()
        .map(|value| value.to_ascii_uppercase())
        .ok_or_else(|| AuError::code("E_TAPE", "empty opcode"))?;
    let suffix = &raw[opcode.len_utf8()..];
    match opcode {
        'Y' => {
            let (count_index, nested_index) = if suffix.is_empty() {
                (1, 2)
            } else {
                (usize::MAX, 1)
            };
            let count = if suffix.is_empty() {
                tokens
                    .get(count_index)
                    .ok_or_else(|| AuError::code("E_TAPE", "Y3 OPCODE or Y 3 OPCODE"))?
                    .parse::<u8>()?
            } else {
                suffix.parse::<u8>()?
            };
            if count == 0 || count > batch::MAX_REPEAT {
                return Err(AuError::code(
                    "E_TAPE",
                    format!("repeat count must be 1..{}", batch::MAX_REPEAT),
                ));
            }
            let nested = tokens
                .get(nested_index..)
                .filter(|values| !values.is_empty())
                .ok_or_else(|| AuError::code("E_TAPE", "Y3 requires one opcode"))?;
            let op = parse_op(nested)?;
            if matches!(op, Op::Repeat { .. }) {
                return Err(AuError::code(
                    "E_TAPE",
                    "nested tape repeats are not allowed",
                ));
            }
            Ok(Op::Repeat {
                count,
                op: Box::new(op),
            })
        }
        'D' => {
            let slot = if suffix.is_empty() {
                parse_slot_arg(tokens, 1)?
            } else {
                parse_slot(suffix)?
            };
            let value_index = if suffix.is_empty() { 2 } else { 1 };
            let value = one(tokens, value_index, "D0 VALUE")?;
            Ok(Op::Dict { slot, value })
        }
        'R' => no_args(tokens, "R").map(|_| Op::Reset),
        'F' => {
            let slot = if suffix.is_empty() {
                0
            } else {
                parse_slot(suffix)?
            };
            let selector = one(tokens, 1, "F0 SELECTOR")?;
            validate_selector(&selector)?;
            Ok(Op::Find { slot, selector })
        }
        'T' => Ok(Op::Tap {
            target: one(tokens, 1, "T TARGET")?,
        }),
        'L' => Ok(Op::Long {
            target: one(tokens, 1, "L TARGET")?,
        }),
        'E' => {
            exact_len(tokens, 3..=3, "E TARGET TEXT")?;
            Ok(Op::Set {
                target: tokens[1].clone(),
                text: tokens[2].clone(),
            })
        }
        'S' => {
            let direction = tokens.get(2).cloned().unwrap_or_else(|| "forward".into());
            if !matches!(direction.as_str(), "forward" | "backward") {
                return Err(AuError::code(
                    "E_TAPE",
                    "S direction must be forward|backward",
                ));
            }
            exact_len(tokens, 2..=3, "S TARGET [forward|backward]")?;
            Ok(Op::Scroll {
                target: one(tokens, 1, "S TARGET [forward|backward]")?,
                direction,
            })
        }
        'W' => {
            exact_len(tokens, 2..=3, "W SELECTOR [MS]")?;
            let selector = tokens[1].clone();
            validate_selector(&selector)?;
            let timeout_ms = optional_timeout(tokens, 2)?;
            Ok(Op::Wait {
                selector,
                timeout_ms,
            })
        }
        'A' => {
            exact_len(tokens, 2..=3, "A SELECTOR [MS]")?;
            let selector = tokens[1].clone();
            validate_selector(&selector)?;
            let timeout_ms = optional_timeout(tokens, 2)?;
            Ok(Op::Assert {
                selector,
                timeout_ms,
            })
        }
        'P' => {
            exact_len(tokens, 3..=4, "P SELECTOR POSTSELECTOR [MS]")?;
            let selector = tokens[1].clone();
            let postcondition = tokens[2].clone();
            validate_selector(&selector)?;
            validate_selector(&postcondition)?;
            Ok(Op::Proof {
                selector,
                postcondition,
                timeout_ms: optional_timeout(tokens, 3)?,
            })
        }
        'K' => Ok(Op::Key {
            key: one(tokens, 1, "K KEY")?,
        }),
        'H' => no_args(tokens, "H").map(|_| Op::Home),
        'B' => no_args(tokens, "B").map(|_| Op::Back),
        'G' => {
            exact_len(tokens, 3..=3, "G X Y")?;
            Ok(Op::TapAt {
                x: tokens[1].clone(),
                y: tokens[2].clone(),
            })
        }
        'Q' => no_args(tokens, "Q").map(|_| Op::Frontier),
        _ => Err(AuError::code(
            "E_TAPE",
            format!("unknown opcode {raw}; use D R F T L E S W A P K H B G Q Y"),
        )),
    }
}

fn is_state_changing(op: &Op) -> bool {
    matches!(
        op,
        Op::Tap { .. }
            | Op::Long { .. }
            | Op::Set { .. }
            | Op::Scroll { .. }
            | Op::Proof { .. }
            | Op::Key { .. }
            | Op::Home
            | Op::Back
            | Op::TapAt { .. }
            | Op::Repeat { .. }
    )
}

fn one(tokens: &[String], index: usize, usage: &str) -> Result<String> {
    exact_len(tokens, index + 1..=index + 1, usage)?;
    tokens
        .get(index)
        .cloned()
        .ok_or_else(|| AuError::code("E_TAPE", usage))
}

fn no_args(tokens: &[String], usage: &str) -> Result<()> {
    exact_len(tokens, 1..=1, usage)
}

fn exact_len(tokens: &[String], range: std::ops::RangeInclusive<usize>, usage: &str) -> Result<()> {
    if range.contains(&tokens.len()) {
        Ok(())
    } else {
        Err(AuError::code("E_TAPE", usage))
    }
}

fn optional_timeout(tokens: &[String], index: usize) -> Result<u64> {
    tokens
        .get(index)
        .map(|value| value.parse::<u64>())
        .transpose()?
        .map(|value| value.clamp(1, 30_000))
        .map_or(Ok(5_000), Ok)
}

fn parse_slot_arg(tokens: &[String], index: usize) -> Result<u8> {
    parse_slot(
        tokens
            .get(index)
            .ok_or_else(|| AuError::code("E_TAPE", "missing dictionary/register slot"))?,
    )
}

fn parse_slot(value: &str) -> Result<u8> {
    let slot = value
        .strip_prefix('@')
        .or_else(|| value.strip_prefix('$'))
        .unwrap_or(value)
        .parse::<u8>()?;
    validate_slot(slot)?;
    Ok(slot)
}

fn validate_slot(slot: u8) -> Result<()> {
    if usize::from(slot) < MAX_DICTIONARY_ENTRIES {
        Ok(())
    } else {
        Err(AuError::code(
            "E_TAPE",
            format!("slot {slot} is outside 0..31"),
        ))
    }
}

fn validate_selector(selector: &str) -> Result<()> {
    if selector.starts_with('@') || selector.starts_with('$') {
        return Ok(());
    }
    Selector::parse(selector).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{disassemble, parse, Op, TapeSession, TAPE_VERSION};

    #[test]
    fn parses_compact_tape_and_preserves_quoted_unicode() {
        let program =
            parse("D0 'text~TAP TARGET,clickable=true#0'; F0 @0; T $0; E $0 'héllo'; P @0 @0 1200")
                .expect("tape");
        assert_eq!(program.ops.len(), 5);
        assert!(matches!(program.ops[0], Op::Dict { slot: 0, .. }));
        assert!(matches!(program.ops[1], Op::Find { slot: 0, .. }));
        assert!(matches!(program.ops[2], Op::Tap { .. }));
    }

    #[test]
    fn dictionary_epoch_checksum_and_reset_are_explicit() {
        let mut session = TapeSession::default();
        let initial_epoch = session.epoch;
        session.define(0, "text~Ready".into()).expect("define");
        let checksum = session.checksum();
        assert_ne!(checksum, "811c9dc5");
        assert_eq!(session.resolve("@0").expect("resolve"), "text~Ready");
        assert!(session.epoch > initial_epoch);
        session.reset();
        assert!(session.resolve("@0").is_err());
    }

    #[test]
    fn tape_rejects_ambiguous_or_unbounded_programs() {
        assert!(parse("T 1 2").is_err());
        let many = (0..=20).map(|_| "H").collect::<Vec<_>>().join(";");
        assert!(parse(&many).is_err());
    }

    #[test]
    fn bounded_repeat_expands_before_execution_limits() {
        let program = parse("Y3 H; Y 2 B").expect("repeat tape");
        assert_eq!(program.ops.len(), 5);
        assert!(program
            .ops
            .iter()
            .all(|op| !matches!(op, Op::Repeat { .. })));
        assert!(parse("Y21 H").is_err());
        assert!(parse("Y20 H; H").is_err());
        assert!(parse("Y3 Y2 H").is_err());
    }

    #[test]
    fn disassembler_uses_the_execution_parser_and_expands_repeats() {
        let decoded = disassemble("D0 'text~A\\'B'; Y2 H; G 50% 25%").expect("disassemble");
        assert_eq!(decoded.version, TAPE_VERSION);
        assert!(decoded.expanded);
        assert_eq!(decoded.instructions, 4);
        assert_eq!(decoded.state_actions, 3);
        assert_eq!(decoded.lines[0], "00 D0 'text~A\\'B'");
        assert_eq!(decoded.lines[1], "01 H");
        assert_eq!(decoded.lines[2], "02 H");
        assert_eq!(decoded.lines[3], "03 G 50% 25%");
    }
}

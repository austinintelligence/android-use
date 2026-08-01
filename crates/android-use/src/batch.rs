use std::time::Duration;

use crate::adb::{fixed_shell_command, shell_quote};
use crate::error::{AuError, Result};

pub const MAX_INSTRUCTIONS: usize = 64;
pub const MAX_STATE_ACTIONS: usize = 20;
pub const MAX_RETRIES: u8 = 2;
pub const MAX_REPEAT: u8 = 20;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchAction {
    pub command: String,
    pub args: Vec<String>,
    pub retries: u8,
    pub repeat: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Boundary {
    Shell,
    Semantic,
    Binary,
    Protocol,
}

/// Tokenize a bounded semicolon/newline program without assigning meaning to
/// its command vocabulary. The model tape reuses this exact quote, escape,
/// and comment grammar while applying its own typed opcode table.
pub fn tokenize_statements(input: &str) -> Result<Vec<Vec<String>>> {
    split_statements(input)?
        .into_iter()
        .map(|statement| tokenize(&statement))
        .collect()
}

pub fn parse(input: &str) -> Result<Vec<BatchAction>> {
    let mut actions = Vec::new();
    let mut state_actions = 0usize;
    for tokens in tokenize_statements(input)? {
        if tokens.is_empty() {
            continue;
        }
        let mut retries = 0;
        let mut repeat = 1;
        let mut offset = 0;
        while let Some(prefix) = tokens.get(offset).map(String::as_str) {
            match prefix {
                "retry" => {
                    if retries != 0 {
                        return Err(AuError::code("E_BATCH", "retry may appear only once"));
                    }
                    let count = tokens
                        .get(offset + 1)
                        .ok_or_else(|| {
                            AuError::code("E_BATCH", "retry requires a count and command")
                        })?
                        .parse::<u8>()?;
                    if count == 0 {
                        return Err(AuError::code(
                            "E_BATCH",
                            "retry count must be greater than zero",
                        ));
                    }
                    if count > MAX_RETRIES {
                        return Err(AuError::code(
                            "E_BATCH",
                            format!("retry count may not exceed {MAX_RETRIES}"),
                        ));
                    }
                    retries = count;
                    offset += 2;
                }
                "repeat" => {
                    if repeat != 1 {
                        return Err(AuError::code("E_BATCH", "repeat may appear only once"));
                    }
                    let count = tokens
                        .get(offset + 1)
                        .ok_or_else(|| {
                            AuError::code("E_BATCH", "repeat requires a count and command")
                        })?
                        .parse::<u8>()?;
                    if count == 0 {
                        return Err(AuError::code(
                            "E_BATCH",
                            "repeat count must be greater than zero",
                        ));
                    }
                    if count > MAX_REPEAT {
                        return Err(AuError::code(
                            "E_BATCH",
                            format!("repeat count may not exceed {MAX_REPEAT}"),
                        ));
                    }
                    repeat = count;
                    offset += 2;
                }
                _ => break,
            }
        }
        let command = tokens
            .get(offset)
            .cloned()
            .ok_or_else(|| AuError::code("E_BATCH", "missing action after retry"))?;
        if command == "if" {
            if tokens.len() < offset + 4 || tokens[offset + 2] != "then" {
                return Err(AuError::code(
                    "E_BATCH",
                    "if syntax is: if ui:SELECTOR then ACTION [ARGS]",
                ));
            }
            let selector = tokens[offset + 1]
                .strip_prefix("ui:")
                .filter(|value| !value.is_empty())
                .ok_or_else(|| AuError::code("E_BATCH", "if condition must start with ui:"))?;
            crate::selector::Selector::parse(selector)?;
            let mut args = vec![selector.into(), tokens[offset + 3].clone()];
            args.extend(tokens.into_iter().skip(offset + 4));
            let action = BatchAction {
                command,
                args,
                retries,
                repeat,
            };
            add_bounded_action(&mut actions, &mut state_actions, action)?;
            continue;
        }
        let action = BatchAction {
            command,
            args: tokens.into_iter().skip(offset + 1).collect(),
            retries,
            repeat,
        };
        add_bounded_action(&mut actions, &mut state_actions, action)?;
    }
    if actions.is_empty() {
        return Err(AuError::code("E_BATCH", "batch is empty"));
    }
    Ok(actions)
}

fn add_bounded_action(
    actions: &mut Vec<BatchAction>,
    state_actions: &mut usize,
    action: BatchAction,
) -> Result<()> {
    if actions.len() >= MAX_INSTRUCTIONS {
        return Err(AuError::code(
            "E_BATCH",
            format!("batch may not exceed {MAX_INSTRUCTIONS} instructions"),
        ));
    }
    if is_state_changing(&action) {
        *state_actions = (*state_actions)
            .checked_add(usize::from(action.repeat) * (usize::from(action.retries) + 1))
            .ok_or_else(|| AuError::code("E_BATCH", "batch action count overflow"))?;
        if *state_actions > MAX_STATE_ACTIONS {
            return Err(AuError::code(
                "E_BATCH",
                format!(
                    "batch may not execute more than {MAX_STATE_ACTIONS} state-changing actions"
                ),
            ));
        }
    }
    actions.push(action);
    Ok(())
}

fn is_state_changing(action: &BatchAction) -> bool {
    let operation = if action.command == "if" {
        action.args.get(1).map(String::as_str)
    } else if matches!(
        action.command.as_str(),
        "ui" | "web" | "app" | "cam" | "mic" | "loc"
    ) {
        action.args.first().map(String::as_str)
    } else {
        Some(action.command.as_str())
    };
    !matches!(
        operation,
        None | Some(
            "find"
                | "snap"
                | "wait"
                | "assert"
                | "watch"
                | "tabs"
                | "text"
                | "list"
                | "info"
                | "perm"
                | "status"
                | "get"
        )
    )
}

pub fn boundary(action: &BatchAction) -> Boundary {
    if action.command == "if" {
        return Boundary::Protocol;
    }
    if matches!(action.command.as_str(), "wait" | "w" | "assert")
        && action
            .args
            .first()
            .is_some_and(|value| value.starts_with("ui:"))
    {
        return Boundary::Semantic;
    }

    // Only these families have a complete lowering into the persistent remote
    // shell. `app start/stop` are intentionally included because they lower to
    // exact, quoted `monkey`/`am force-stop` calls. Every other structured
    // family must cross a typed protocol boundary so it cannot accidentally be
    // interpreted as shell text or be rejected by lower_shell_action.
    let shell_compatible = matches!(
        action.command.as_str(),
        "t" | "tap"
            | "dt"
            | "lp"
            | "long"
            | "sw"
            | "swipe"
            | "dr"
            | "drag"
            | "tx"
            | "text"
            | "k"
            | "key"
            | "home"
            | "back"
            | "recents"
            | "notify"
            | "quick"
            | "wake"
            | "sleep"
            | "rot"
            | "wait"
            | "w"
    ) || (action.command == "app"
        && matches!(
            action.args.first().map(String::as_str),
            Some("start" | "stop")
        ));
    if shell_compatible {
        return Boundary::Shell;
    }

    if matches!(
        action.command.as_str(),
        "ui" | "vision"
            | "ss"
            | "screenshot"
            | "cam"
            | "mic"
            | "mirror"
            | "screen"
            | "web"
            | "app"
            | "loc"
            | "clip"
            | "notif"
            | "file"
            | "prop"
            | "settings"
            | "sys"
            | "log"
            | "ps"
            | "fwd"
            | "rev"
            | "adb"
            | "sh"
            | "d"
            | "devices"
            | "u"
            | "use"
            | "p"
            | "pair"
            | "c"
            | "connect"
            | "dc"
            | "disconnect"
            | "st"
            | "status"
            | "cap"
            | "doctor"
            | "daemon"
            | "pipe"
            | "tape"
            | "x"
            | "exp"
            | "b"
            | "batch"
    ) {
        return Boundary::Protocol;
    }

    // Preserve the existing E_BATCH diagnostic for unknown commands by
    // allowing lower_shell_action to reject them rather than silently routing
    // arbitrary vocabulary through the structured dispatcher.
    Boundary::Shell
}

/// Convert the DSL shorthand `wait ui:SELECTOR [MS]` / `assert ui:SELECTOR`
/// into the public semantic UI command without treating selector data as shell.
pub fn semantic_shorthand(action: &BatchAction) -> Option<BatchAction> {
    if !matches!(action.command.as_str(), "wait" | "w" | "assert") {
        return None;
    }
    let selector = action.args.first()?.strip_prefix("ui:")?;
    let mut args = Vec::with_capacity(action.args.len() + 1);
    args.push(if action.command == "assert" {
        "assert".into()
    } else {
        "wait".into()
    });
    args.push(selector.into());
    args.extend(action.args.iter().skip(1).cloned());
    Some(BatchAction {
        command: "ui".into(),
        args,
        retries: action.retries,
        repeat: action.repeat,
    })
}

pub fn lower_shell(actions: &[BatchAction]) -> Result<String> {
    lower_shell_with_delay(actions, 0, false)
}

/// Lower a contiguous shell-compatible run and insert a bounded pacing gap
/// before every action after the first. `leading_delay` is used when this run
/// follows a protocol/semantic boundary in the surrounding batch.
pub fn lower_shell_with_delay(
    actions: &[BatchAction],
    delay_ms: u64,
    leading_delay: bool,
) -> Result<String> {
    let mut lines = Vec::with_capacity(actions.len());
    // A gap is needed after a state-changing input, not after a no-op or an
    // explicit remote wait. This keeps the user-friendly 250 ms fallback for
    // GUI mutations without taxing read-like batch probes.
    let mut pending_settle = leading_delay;
    for action in actions {
        if boundary(action) != Boundary::Shell {
            return Err(AuError::code(
                "E_BATCH",
                "non-shell batch action crossed into shell lowering",
            ));
        }
        let command = lower_shell_action(action)?;
        let state_changing = needs_settle(action);
        for _ in 0..action.repeat {
            if state_changing && pending_settle && delay_ms > 0 {
                let seconds = Duration::from_millis(delay_ms).as_secs_f64();
                lines.push(format!("sleep {seconds:.3}"));
            }
            lines.push(lower_retry_chain(
                &command,
                action.retries,
                if state_changing { delay_ms } else { 0 },
            ));
            if is_positive_wait(action) {
                pending_settle = false;
            } else if state_changing {
                pending_settle = true;
            }
        }
    }
    // A batch is a bounded program, not a best-effort list. Stop at the first
    // failed logical action so a later success cannot mask an earlier failure.
    // Retry short-circuiting is already contained inside each action chain.
    Ok(lines.join(" && "))
}

fn needs_settle(action: &BatchAction) -> bool {
    !matches!(action.command.as_str(), "w" | "wait")
}

fn is_positive_wait(action: &BatchAction) -> bool {
    matches!(action.command.as_str(), "w" | "wait")
        && action
            .args
            .first()
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|value| value > 0)
}

/// Build a shell-native retry chain that stops after the first successful
/// attempt. Each attempt is grouped so compound actions (for example the
/// double-tap lowering) cannot leak their internal semicolon into the retry
/// control flow. A failed final attempt remains the transaction's exit status.
fn lower_retry_chain(command: &str, retries: u8, delay_ms: u64) -> String {
    if retries == 0 {
        return command.into();
    }
    let attempt = |value: &str| format!("{{ {value}; }}");
    let mut chain = attempt(command);
    for _ in 0..retries {
        let next = attempt(command);
        if delay_ms > 0 {
            let seconds = Duration::from_millis(delay_ms).as_secs_f64();
            chain.push_str(&format!(" || (sleep {seconds:.3}; {next})"));
        } else {
            chain.push_str(&format!(" || {next}"));
        }
    }
    chain
}

fn lower_shell_action(action: &BatchAction) -> Result<String> {
    let args = &action.args;
    match action.command.as_str() {
        "t" | "tap" => exactly(args, 2, "tap").map(|_| {
            fixed_shell_command(&[
                "input".into(),
                "tap".into(),
                args[0].clone(),
                args[1].clone(),
            ])
        }),
        "dt" => exactly(args, 2, "dt").map(|_| {
            format!(
                "{}; {}",
                fixed_shell_command(&[
                    "input".into(),
                    "tap".into(),
                    args[0].clone(),
                    args[1].clone()
                ]),
                fixed_shell_command(&[
                    "input".into(),
                    "tap".into(),
                    args[0].clone(),
                    args[1].clone()
                ])
            )
        }),
        "lp" | "long" => exactly(args, 2, "lp").map(|_| {
            fixed_shell_command(&[
                "input".into(),
                "swipe".into(),
                args[0].clone(),
                args[1].clone(),
                args[0].clone(),
                args[1].clone(),
                "650".into(),
            ])
        }),
        "sw" | "swipe" => {
            if !(4..=5).contains(&args.len()) {
                return Err(AuError::code("E_ARGS", "sw requires X1 Y1 X2 Y2 [MS]"));
            }
            let mut command = vec!["input".into(), "swipe".into()];
            command.extend(args.iter().cloned());
            Ok(fixed_shell_command(&command))
        }
        "dr" | "drag" => lower_shell_action(&BatchAction {
            command: "sw".into(),
            args: args.clone(),
            retries: 0,
            repeat: 1,
        }),
        "tx" | "text" => {
            exactly(args, 1, "tx")?;
            Ok(format!(
                "input text {}",
                shell_quote(&encode_input_text(&args[0]))
            ))
        }
        "k" | "key" => exactly(args, 1, "k")
            .map(|_| fixed_shell_command(&["input".into(), "keyevent".into(), args[0].clone()])),
        "home" => Ok("input keyevent KEYCODE_HOME".into()),
        "back" => Ok("input keyevent KEYCODE_BACK".into()),
        "recents" => Ok("input keyevent KEYCODE_APP_SWITCH".into()),
        "notify" => Ok("cmd statusbar expand-notifications".into()),
        "quick" => Ok("cmd statusbar expand-settings".into()),
        "wake" => Ok("input keyevent KEYCODE_WAKEUP".into()),
        "sleep" => Ok("input keyevent KEYCODE_SLEEP".into()),
        "rot" => exactly(args, 1, "rot").map(|_| {
            fixed_shell_command(&[
                "settings".into(),
                "put".into(),
                "system".into(),
                "user_rotation".into(),
                args[0].clone(),
            ])
        }),
        "w" | "wait" => {
            exactly(args, 1, "wait")?;
            let milliseconds: u64 = args[0].parse()?;
            if milliseconds > 30_000 {
                return Err(AuError::code(
                    "E_ARGS",
                    "batch wait may not exceed 30000 ms",
                ));
            }
            if milliseconds == 0 {
                // `sleep 0` forks a remote process on Android. A zero wait is
                // a true no-op and must stay inside the persistent shell so
                // the fast-path benchmark measures transport, not a needless
                // child-process launch.
                return Ok(":".into());
            }
            let seconds = Duration::from_millis(milliseconds).as_secs_f64();
            Ok(format!("sleep {seconds:.3}"))
        }
        "app" if args.first().is_some_and(|value| value == "start") => {
            let package = args
                .get(1)
                .ok_or_else(|| AuError::code("E_ARGS", "app start requires package"))?;
            Ok(fixed_shell_command(&[
                "monkey".into(),
                "-p".into(),
                package.clone(),
                "1".into(),
            ]))
        }
        "app" if args.first().is_some_and(|value| value == "stop") => {
            let package = args
                .get(1)
                .ok_or_else(|| AuError::code("E_ARGS", "app stop requires package"))?;
            Ok(fixed_shell_command(&[
                "am".into(),
                "force-stop".into(),
                package.clone(),
            ]))
        }
        _ => Err(AuError::code(
            "E_BATCH",
            format!("{} is not shell-compatible", action.command),
        )),
    }
}

fn exactly(args: &[String], count: usize, name: &str) -> Result<()> {
    if args.len() == count {
        Ok(())
    } else {
        Err(AuError::code(
            "E_ARGS",
            format!("{name} requires {count} arguments"),
        ))
    }
}

fn encode_input_text(value: &str) -> String {
    value.replace([' ', '\n'], "%s").replace('&', "\\&")
}

fn split_statements(input: &str) -> Result<Vec<String>> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    for character in input.chars() {
        if comment {
            if character == '\n' {
                comment = false;
                if !current.trim().is_empty() {
                    statements.push(std::mem::take(&mut current));
                }
            }
            continue;
        }
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            current.push(character);
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if character == active {
                quote = None;
            }
            current.push(character);
            continue;
        }
        if matches!(character, '\'' | '"') {
            quote = Some(character);
            current.push(character);
        } else if character == '#' && current.trim().is_empty() {
            comment = true;
        } else if matches!(character, ';' | '\n') {
            if !current.trim().is_empty() {
                statements.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if quote.is_some() || escaped {
        return Err(AuError::code("E_BATCH", "unterminated quote or escape"));
    }
    if !current.trim().is_empty() {
        statements.push(current);
    }
    Ok(statements)
}

pub fn tokenize(input: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in input.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if let Some(active) = quote {
            if character == active {
                quote = None;
            } else {
                current.push(character);
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if quote.is_some() || escaped {
        return Err(AuError::code("E_BATCH", "unterminated quote or escape"));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::{
        boundary, lower_shell, lower_shell_with_delay, parse, semantic_shorthand, Boundary,
    };

    #[test]
    fn parses_quotes_comments_and_semicolons() {
        let actions =
            parse("# comment\nt 50% 10%; tx 'hello world'; retry 2 k HOME").expect("batch");
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[1].args, ["hello world"]);
        assert_eq!(actions[2].retries, 2);
        assert_eq!(actions[2].repeat, 1);
    }

    #[test]
    fn shell_lowering_quotes_user_text() {
        let actions = parse("tx 'x; $(id)'").expect("batch");
        let lowered = lower_shell(&actions).expect("lower");
        assert!(lowered.contains("'x;%s$(id)'"));
    }

    #[test]
    fn semantic_wait_stays_out_of_shell_lowering() {
        let action = parse("wait ui:text~Ready 1000").expect("batch").remove(0);
        let mapped = semantic_shorthand(&action).expect("semantic shorthand");
        assert_eq!(mapped.command, "ui");
        assert_eq!(mapped.args, ["wait", "text~Ready", "1000"]);
    }

    #[test]
    fn structured_families_cross_protocol_boundaries() {
        assert_eq!(
            boundary(&parse("app ls").expect("app")[0]),
            Boundary::Protocol
        );
        assert_eq!(
            boundary(&parse("app start dev.codex.aubridge").expect("app start")[0]),
            Boundary::Shell
        );
        assert_eq!(
            boundary(&parse("clip").expect("clip")[0]),
            Boundary::Protocol
        );
        assert_eq!(
            boundary(&parse("file pull /sdcard/a").expect("file")[0]),
            Boundary::Protocol
        );
        assert_eq!(
            boundary(&parse("adb -- shell id").expect("adb")[0]),
            Boundary::Protocol
        );
    }

    #[test]
    fn pacing_is_between_actions_and_can_start_after_a_boundary() {
        let actions = parse("home; back").expect("batch");
        let lowered = lower_shell_with_delay(&actions, 250, false).expect("lower");
        assert_eq!(lowered.matches("sleep 0.250").count(), 1);
        assert!(!lowered.starts_with("sleep"));

        let continued = lower_shell_with_delay(&actions[0..1], 200, true).expect("lower");
        assert!(continued.starts_with("sleep 0.200 && "));
    }

    #[test]
    fn parses_bounded_semantic_condition() {
        let actions = parse("if ui:text~Ready then t 50% 50%").expect("batch");
        assert_eq!(actions[0].command, "if");
        assert_eq!(actions[0].args, ["text~Ready", "t", "50%", "50%"]);
        assert_eq!(boundary(&actions[0]), Boundary::Protocol);
    }

    #[test]
    fn parses_quoted_and_escaped_condition_selectors() {
        let quoted = parse("if ui:\"text~TAP TARGET#0\" then ui tap \"desc~AU tap target#0\"")
            .expect("quoted condition");
        assert_eq!(
            quoted[0].args,
            ["text~TAP TARGET#0", "ui", "tap", "desc~AU tap target#0"]
        );
        let escaped = parse(r#"if ui:text~TAP\ TARGET#0 then ui tap desc~AU\ tap\ target#0"#)
            .expect("escaped condition");
        assert_eq!(escaped[0].args, quoted[0].args);
    }

    #[test]
    fn retries_are_retries_after_the_first_attempt_and_are_bounded() {
        let actions = parse("retry 2 home").expect("retry");
        assert_eq!(actions[0].retries, 2);
        let lowered = lower_shell(&actions).expect("lower");
        assert_eq!(lowered.matches("KEYCODE_HOME").count(), 3);
        assert!(lowered.contains(" || "));
        assert!(lowered.contains("&&") || lowered.starts_with("{"));

        let error = parse("retry 3 home").expect_err("retry bound");
        assert_eq!(error.kind(), "E_BATCH");
    }

    #[test]
    fn zero_wait_is_a_remote_shell_builtin_noop() {
        let actions = parse("w 0").expect("wait");
        assert_eq!(lower_shell(&actions).expect("lower"), ":");
    }

    #[test]
    fn default_pacing_skips_noops_but_keeps_gui_settle_gaps() {
        let actions = parse("home; w 0; back").expect("batch");
        let lowered = lower_shell_with_delay(&actions, 250, false).expect("lower");
        assert_eq!(lowered.matches("sleep 0.250").count(), 1);
        assert!(lowered.contains("KEYCODE_HOME"));
        assert!(lowered.contains("KEYCODE_BACK"));

        let waited = parse("home; w 100; back").expect("batch");
        let lowered = lower_shell_with_delay(&waited, 250, false).expect("lower");
        assert_eq!(lowered.matches("sleep 0.250").count(), 0);
        assert!(lowered.contains("sleep 0.100"));
    }

    #[test]
    fn state_changing_and_instruction_limits_are_bounded() {
        let too_many_state_actions = (0..21).map(|_| "home").collect::<Vec<_>>().join(";");
        let error = parse(&too_many_state_actions).expect_err("state bound");
        assert_eq!(error.kind(), "E_BATCH");

        let too_many_queries = (0..65)
            .map(|_| "ui find text~Ready")
            .collect::<Vec<_>>()
            .join(";");
        let error = parse(&too_many_queries).expect_err("instruction bound");
        assert_eq!(error.kind(), "E_BATCH");
    }

    #[test]
    fn repeat_is_intentional_and_bounded_before_execution() {
        let actions = parse("repeat 3 home").expect("repeat");
        assert_eq!(actions[0].repeat, 3);
        let lowered = lower_shell(&actions).expect("lower");
        assert_eq!(lowered.matches("KEYCODE_HOME").count(), 3);
        assert!(parse("repeat 21 home").is_err());
        assert!(parse("repeat 20 retry 2 home").is_err());
    }

    #[test]
    fn retry_chain_does_not_retry_after_success() {
        let actions = parse("retry 2 home").expect("retry");
        let lowered = lower_shell_with_delay(&actions, 250, false).expect("lower");
        assert_eq!(lowered.matches("KEYCODE_HOME").count(), 3);
        assert_eq!(lowered.matches("sleep 0.250").count(), 2);
    }

    #[test]
    fn shell_batch_stops_after_a_failed_logical_action() {
        let actions = parse("home; back").expect("batch");
        let lowered = lower_shell(&actions).expect("lower");
        assert!(lowered.contains("KEYCODE_HOME") && lowered.contains("KEYCODE_BACK"));
        assert!(lowered.contains("&&"));
        assert!(!lowered.contains("; input keyevent"));
    }
}

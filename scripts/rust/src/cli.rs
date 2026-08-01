use std::path::PathBuf;

use crate::device::Endpoint;
use crate::error::{AuError, Result};
use crate::output::OutputMode;

pub const DEFAULT_BATCH_DELAY_MS: u64 = 250;
pub const MAX_BATCH_DELAY_MS: u64 = 999;

#[derive(Debug)]
struct ParseState {
    serial: Option<String>,
    timeout_ms: u64,
    batch_delay_ms: u64,
    batch_delay_explicit: bool,
    output: OutputMode,
    force: bool,
    output_path: Option<PathBuf>,
    no_daemon: bool,
    daemon_child: bool,
    pipe_jsonl: bool,
    disassemble: bool,
    trace_path: Option<PathBuf>,
    trace_id: Option<String>,
}

impl Default for ParseState {
    fn default() -> Self {
        Self {
            serial: None,
            timeout_ms: 8_000,
            batch_delay_ms: DEFAULT_BATCH_DELAY_MS,
            batch_delay_explicit: false,
            output: OutputMode::default(),
            force: false,
            output_path: None,
            no_daemon: false,
            daemon_child: false,
            pipe_jsonl: false,
            disassemble: false,
            trace_path: None,
            trace_id: None,
        }
    }
}

impl ParseState {
    /// Consume one execution option and return the next argv index. Unknown
    /// tokens are deliberately left to the command grammar. This makes the
    /// parser composable while preserving exact raw `adb`/`sh` argv.
    fn consume(&mut self, tokens: &[String], index: usize) -> Result<Option<usize>> {
        let token = tokens.get(index).map(String::as_str).unwrap_or_default();
        let next_value = |name: &str| required_flag_value(tokens, index + 1, name);
        match token {
            "-j" | "--json" => {
                self.output.json = true;
                Ok(Some(index + 1))
            }
            "-c" | "--compact" => {
                self.output.compact = true;
                Ok(Some(index + 1))
            }
            "-q" | "--quiet" => {
                self.output.quiet = true;
                Ok(Some(index + 1))
            }
            "-w" | "--wire" => {
                self.output.wire = true;
                Ok(Some(index + 1))
            }
            "--binary" => {
                self.output.binary = true;
                Ok(Some(index + 1))
            }
            "--force" => {
                self.force = true;
                Ok(Some(index + 1))
            }
            "--no-daemon" => {
                self.no_daemon = true;
                Ok(Some(index + 1))
            }
            "--daemon-child" => {
                self.daemon_child = true;
                Ok(Some(index + 1))
            }
            "--jsonl" => {
                self.pipe_jsonl = true;
                Ok(Some(index + 1))
            }
            "--disasm" | "--disassemble" | "--decode" => {
                self.disassemble = true;
                Ok(Some(index + 1))
            }
            "--trace" => {
                self.trace_path = Some(PathBuf::from(next_value("trace")?));
                Ok(Some(index + 2))
            }
            "--trace-id" => {
                self.trace_id = Some(next_value("trace-id")?);
                Ok(Some(index + 2))
            }
            "-s" | "--serial" => {
                self.serial = Some(next_value("serial")?);
                Ok(Some(index + 2))
            }
            "--timeout" => {
                self.timeout_ms = next_value("timeout")?.parse()?;
                if self.timeout_ms == 0 || self.timeout_ms > 600_000 {
                    return Err(AuError::code("E_ARGS", "timeout must be 1..600000 ms"));
                }
                Ok(Some(index + 2))
            }
            "--batch-delay" | "--delay" => {
                self.batch_delay_explicit = true;
                self.batch_delay_ms = next_value("batch-delay")?.parse()?;
                if self.batch_delay_ms > MAX_BATCH_DELAY_MS {
                    return Err(AuError::code("E_ARGS", "batch delay must be 0..999 ms"));
                }
                Ok(Some(index + 2))
            }
            "--out" => {
                self.output_path = Some(PathBuf::from(next_value("out")?));
                Ok(Some(index + 2))
            }
            _ => Ok(None),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Cli {
    pub command: String,
    pub args: Vec<String>,
    pub serial: Option<String>,
    pub timeout_ms: u64,
    pub batch_delay_ms: u64,
    pub batch_delay_explicit: bool,
    pub output: OutputMode,
    pub force: bool,
    pub output_path: Option<PathBuf>,
    pub no_daemon: bool,
    pub daemon_child: bool,
    pub pipe_jsonl: bool,
    pub disassemble: bool,
    pub trace_path: Option<PathBuf>,
    pub trace_id: Option<String>,
    /// Validated only by the dispatcher for one batch/tape transaction.
    pub resolved_endpoint: Option<Endpoint>,
}

impl Cli {
    pub fn parse<I>(arguments: I) -> Result<Self>
    where
        I: IntoIterator<Item = String>,
    {
        let tokens: Vec<String> = arguments.into_iter().collect();
        if matches!(tokens.as_slice(), [value] if matches!(value.as_str(), "--help" | "-h" | "--version"))
        {
            return Ok(Self {
                command: tokens[0].clone(),
                args: Vec::new(),
                serial: None,
                timeout_ms: 8_000,
                batch_delay_ms: DEFAULT_BATCH_DELAY_MS,
                batch_delay_explicit: false,
                output: OutputMode::default(),
                force: false,
                output_path: None,
                no_daemon: true,
                daemon_child: false,
                pipe_jsonl: false,
                disassemble: false,
                trace_path: None,
                trace_id: None,
                resolved_endpoint: None,
            });
        }
        let mut index = 0usize;
        let mut state = ParseState::default();

        while index < tokens.len() {
            if let Some(next) = state.consume(&tokens, index)? {
                index = next;
            } else {
                if matches!(tokens[index].as_str(), "--help" | "-h" | "--version") {
                    break;
                }
                if tokens[index].starts_with('-') {
                    return Err(AuError::code(
                        "E_ARGS",
                        format!("unknown option {}", tokens[index]),
                    ));
                }
                break;
            }
        }

        let command = tokens.get(index).cloned().unwrap_or_else(|| "help".into());
        let raw_command = matches!(command.as_str(), "adb" | "sh");
        let mut args = Vec::new();
        let mut stop_options = false;
        index = index.saturating_add(1);
        while index < tokens.len() {
            if !raw_command && !stop_options {
                if tokens[index] == "--" {
                    stop_options = true;
                    args.push(tokens[index].clone());
                    index += 1;
                    continue;
                }
                if let Some(next) = state.consume(&tokens, index)? {
                    index = next;
                    continue;
                }
            }
            args.push(tokens[index].clone());
            index += 1;
        }
        Ok(Self {
            command,
            args,
            serial: state.serial,
            timeout_ms: state.timeout_ms,
            batch_delay_ms: state.batch_delay_ms,
            batch_delay_explicit: state.batch_delay_explicit,
            output: state.output,
            force: state.force,
            output_path: state.output_path,
            no_daemon: state.no_daemon,
            daemon_child: state.daemon_child,
            pipe_jsonl: state.pipe_jsonl,
            disassemble: state.disassemble,
            trace_path: state.trace_path,
            trace_id: state.trace_id,
            resolved_endpoint: None,
        })
    }

    pub fn should_use_daemon(&self) -> bool {
        if self.no_daemon || self.daemon_child || self.disassemble {
            return false;
        }
        // Long-running and privacy-sensitive transactions must remain owned by
        // the client process. If that client disappears, the helper heartbeat
        // and media child can observe the disconnect instead of leaving a
        // daemon request stranded until its deadline.
        if matches!(self.command.as_str(), "mirror")
            || (self.command == "screen" && self.args.first().map(String::as_str) == Some("record"))
            || (self.command == "cam"
                && matches!(
                    self.args.first().map(String::as_str),
                    Some("snap" | "view" | "record" | "pipe")
                ))
            || (self.command == "mic"
                && matches!(self.args.first().map(String::as_str), Some("cap" | "pipe")))
            || (self.command == "loc" && self.args.first().map(String::as_str) == Some("route"))
            || self.command == "vision"
        {
            return false;
        }
        !matches!(
            self.command.as_str(),
            "help"
                | "-h"
                | "--help"
                | "version"
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
                | "daemon"
                | "pipe"
        )
    }

    pub fn is_raw(&self) -> bool {
        matches!(self.command.as_str(), "adb" | "sh")
    }

    /// Build a nested command without re-parsing argv or dropping execution
    /// policy. Batch, tape, and pipe share this constructor.
    pub fn child(
        &self,
        command: impl Into<String>,
        args: Vec<String>,
        endpoint: &Endpoint,
    ) -> Self {
        Self {
            command: command.into(),
            args,
            serial: Some(endpoint.endpoint.clone()),
            timeout_ms: self.timeout_ms,
            batch_delay_ms: self.batch_delay_ms,
            batch_delay_explicit: self.batch_delay_explicit,
            output: self.output,
            force: self.force,
            output_path: self.output_path.clone(),
            no_daemon: true,
            daemon_child: true,
            pipe_jsonl: self.pipe_jsonl,
            disassemble: self.disassemble,
            trace_path: self.trace_path.clone(),
            trace_id: self.trace_id.clone(),
            resolved_endpoint: Some(endpoint.clone()),
        }
    }
}

fn required_flag_value(tokens: &[String], index: usize, name: &str) -> Result<String> {
    tokens
        .get(index)
        .cloned()
        .ok_or_else(|| AuError::code("E_ARGS", format!("--{name} requires a value")))
}

pub fn help_text() -> &'static str {
    "au 1.0\n\nconnection: d u p c dc st cap doctor\nfast: b pipe tape|x daemon start|stop|status|ping\ntape: D0 V; R; F0 SELECTOR; T|L|E|S $0; W|A SELECTOR [MS]; P SELECTOR POST [MS]; K KEY; H B; G X Y; Q; --disasm\nexperiment: exp f1 SELECTOR POSTSELECTOR [TIMEOUT_MS]\ngui: t dt lp sw dr tx k home back recents notify quick wake sleep rot ss\nsemantic: ui snap|find|tap|long|set|scroll|wait|assert|watch|global|gesture\nvision: inspect|hash|diff|crop|region|check|clear\nweb: web open|tabs|use|go|click|type|text|eval|wait|back|reload|close|shot\napps: app ls|info|start|stop|install|uninstall|clear|perm|grant|revoke|intent\nmedia: mirror screen record cam list|view|snap|record|pipe mic cap|pipe\nlocation: loc status|get|set|clear|route|enable|disable\nsystem: clip notif ls|watch|open|action|dismiss file prop settings sys log ps fwd rev\nraw: adb -- ... | adb -g -- ... | sh -- ...\n\nGlobal options may precede or follow normal commands: -s SERIAL|wifi|usb|mdns -j -c -w -q --delay MS --timeout MS --out PATH --force --binary --no-daemon --jsonl --trace PATH --disasm. Raw adb/sh preserve every post-command argument.\n-c/--compact: dense JSON {o:1,d|n|p|t} or {o:0,e,m}; -w/--wire: versioned dense envelope {v:1,o:1,d|n|p|t} or {v:1,o:0,e,m}; -j is stable JSON. Structured output is bounded; use --out for large results.\nTape is bounded to 64 instructions/20 state actions; @N is a session dictionary ref and $N is a tape-run node register.\nBatch control prefixes: `retry 1 ACTION` retries only after a failed shell attempt (semantic retries are limited to read-only/synchronization actions); `repeat N ACTION` intentionally runs N times. Combined worst-case attempts remain bounded by 20 state actions.\nBatch pacing: --delay/--batch-delay 0..999 ms (default 250 for shell actions; semantic proof paths do not pace). Long media and location routes stay foreground for cancellation. `pipe --jsonl` accepts compact request objects {c:COMMAND,a:[ARGS]} or {b:DSL}. `--trace PATH` appends bounded JSONL spans with one propagated trace ID."
}

#[cfg(test)]
mod tests {
    use super::Cli;

    #[test]
    fn parses_global_flags_without_losing_raw_values() {
        let cli = Cli::parse(vec![
            "-j".into(),
            "-s".into(),
            "abc".into(),
            "adb".into(),
            "--".into(),
            "shell".into(),
            "echo $HOME".into(),
        ])
        .expect("parse");
        assert!(cli.output.json);
        assert_eq!(cli.serial.as_deref(), Some("abc"));
        assert_eq!(cli.args, ["--", "shell", "echo $HOME"]);
    }

    #[test]
    fn parses_compact_machine_output_flag() {
        let cli = Cli::parse(vec!["-c".into(), "st".into()]).expect("compact");
        assert!(cli.output.compact);
    }

    #[test]
    fn parses_versioned_wire_output_flag() {
        let cli = Cli::parse(vec!["-w".into(), "st".into()]).expect("wire");
        assert!(cli.output.wire);
    }

    #[test]
    fn parses_trace_options_before_and_after_the_command() {
        let cli = Cli::parse(vec![
            "st".into(),
            "--trace".into(),
            "trace.jsonl".into(),
            "--trace-id".into(),
            "run-1".into(),
        ])
        .expect("trace options");
        assert_eq!(
            cli.trace_path.as_deref(),
            Some(std::path::Path::new("trace.jsonl"))
        );
        assert_eq!(cli.trace_id.as_deref(), Some("run-1"));
    }

    #[test]
    fn parses_disassembler_flag_and_keeps_it_off_the_daemon_path() {
        let cli = Cli::parse(vec!["x".into(), "H; B".into(), "--disasm".into()])
            .expect("disassembler flag");
        assert!(cli.disassemble);
        assert!(!cli.should_use_daemon());
        assert_eq!(cli.args, ["H; B"]);
    }

    #[test]
    fn extracts_execution_flags_after_the_command() {
        let cli = Cli::parse(vec![
            "b".into(),
            "home; back".into(),
            "--delay".into(),
            "200".into(),
            "-c".into(),
        ])
        .expect("post-command flags");
        assert_eq!(cli.args, ["home; back"]);
        assert_eq!(cli.batch_delay_ms, 200);
        assert!(cli.batch_delay_explicit);
        assert!(cli.output.compact);
    }

    #[test]
    fn raw_backend_keeps_every_argument_after_command() {
        let cli = Cli::parse(vec![
            "adb".into(),
            "--".into(),
            "shell".into(),
            "echo".into(),
            "--delay".into(),
            "200".into(),
        ])
        .expect("raw argv");
        assert_eq!(cli.args, ["--", "shell", "echo", "--delay", "200"]);
        assert!(!cli.batch_delay_explicit);
    }

    #[test]
    fn parses_jsonl_pipe_mode() {
        let cli = Cli::parse(vec!["pipe".into(), "--jsonl".into()]).expect("jsonl");
        assert!(cli.pipe_jsonl);
        assert!(cli.args.is_empty());
    }

    #[test]
    fn batch_delay_defaults_and_can_be_overridden_or_disabled() {
        let default = Cli::parse(vec!["b".into(), "home".into()]).expect("default");
        assert_eq!(default.batch_delay_ms, 250);

        let explicit = Cli::parse(vec![
            "--delay".into(),
            "200".into(),
            "b".into(),
            "home".into(),
        ])
        .expect("explicit");
        assert_eq!(explicit.batch_delay_ms, 200);

        let disabled = Cli::parse(vec![
            "--batch-delay".into(),
            "0".into(),
            "b".into(),
            "home".into(),
        ])
        .expect("disabled");
        assert_eq!(disabled.batch_delay_ms, 0);
    }

    #[test]
    fn batch_delay_rejects_multi_second_values() {
        let error = Cli::parse(vec![
            "--delay".into(),
            "1000".into(),
            "b".into(),
            "home".into(),
        ])
        .expect_err("out of range");
        assert_eq!(error.kind(), "E_ARGS");
    }

    #[test]
    fn long_lived_media_and_location_commands_stay_foreground() {
        for raw in [
            vec!["mirror".into()],
            vec!["screen".into(), "record".into(), "1".into()],
            vec!["cam".into(), "record".into()],
            vec!["mic".into(), "cap".into()],
            vec!["loc".into(), "route".into(), "route.csv".into()],
        ] {
            assert!(!Cli::parse(raw).expect("parse").should_use_daemon());
        }
        assert!(Cli::parse(vec!["cam".into(), "list".into()])
            .expect("parse")
            .should_use_daemon());
    }
}

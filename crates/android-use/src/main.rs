use std::env;

use android_use::actions::{self, ActionResult, Brief};
use android_use::cli::{help_text, Cli};
use android_use::config::AppPaths;
use android_use::daemon;
use android_use::error::{AuError, Result};
use android_use::output::{emit_action_result, emit_error};
use android_use::serve;
use android_use::trace;
use android_use::VERSION;

fn main() {
    let mut raw: Vec<String> = env::args().skip(1).collect();
    let cli = match Cli::parse(raw.clone()) {
        Ok(cli) => cli,
        Err(error) => {
            emit_error(Default::default(), &error);
            std::process::exit(2);
        }
    };
    let trace_id = match trace::configure(cli.trace_path.as_deref(), cli.trace_id.as_deref()) {
        Ok(id) => id,
        Err(error) => {
            emit_error(cli.output, &error);
            std::process::exit(2);
        }
    };
    if let (Some(id), true) = (trace_id, cli.trace_path.is_some() && cli.trace_id.is_none()) {
        raw.push("--trace-id".into());
        raw.push(id);
    }
    let _process_span = trace::span(
        "cli.process",
        serde_json::json!({"c":cli.command,"a":cli.args.len()}),
    );
    if cli.command == "pipe" {
        let mode = cli.output;
        match actions::stream_pipe(&cli, |result| {
            match result {
                Ok(result) => emit_action_result(mode, result),
                Err(error) => emit_error(mode, &error),
            }
            Ok(())
        }) {
            Ok(_) => return,
            Err(error) => {
                emit_error(mode, &error);
                std::process::exit(1);
            }
        }
    }
    if cli.command == "serve" {
        if let Err(error) = serve::run(&cli) {
            emit_error(cli.output, &error);
            std::process::exit(1);
        }
        return;
    }
    let result = run(&cli, raw);
    match result {
        Ok(result) => {
            // `--binary` without `--out` is the explicit stream-to-stdout
            // escape hatch.  Once the caller redirected the artifact, return
            // the normal compact proof instead of duplicating private media
            // into the agent transcript.
            if cli.output.binary && cli.output_path.is_none() {
                if let Some(path) = result
                    .data
                    .get("binary_path")
                    .and_then(serde_json::Value::as_str)
                {
                    if let Err(error) = emit_binary(path) {
                        emit_error(cli.output, &error);
                        std::process::exit(1);
                    }
                    return;
                }
            }
            emit_action_result(cli.output, result)
        }
        Err(error) => {
            emit_error(cli.output, &error);
            std::process::exit(1);
        }
    }
}

fn emit_binary(path: &str) -> Result<()> {
    use std::fs::File;
    use std::io::{self, Write};

    let mut input = File::open(path).map_err(|error| {
        AuError::code("E_BINARY", format!("open binary artifact {path}: {error}"))
    })?;
    let mut output = io::stdout().lock();
    io::copy(&mut input, &mut output)
        .map_err(|error| AuError::code("E_BINARY", format!("write binary artifact: {error}")))?;
    output
        .flush()
        .map_err(|error| AuError::code("E_BINARY", format!("flush binary artifact: {error}")))?;
    Ok(())
}

fn run(cli: &Cli, raw: Vec<String>) -> Result<ActionResult> {
    trace::event(
        "cli.dispatch",
        serde_json::json!({"c":cli.command,"a":cli.args.len(),"daemon":cli.should_use_daemon()}),
    );
    match cli.command.as_str() {
        "help" | "-h" | "--help" => Ok(ActionResult::text(
            "android-use; pass -j help for the compact command map",
            serde_json::json!({"help":help_text()}),
        )),
        "version" | "--version" => Ok(ActionResult::text(
            VERSION,
            serde_json::json!({"version":VERSION}),
        )),
        "daemon" => daemon_command(cli),
        _ if cli.should_use_daemon() => {
            let paths = AppPaths::discover()?;
            let value = daemon::execute_or_start(&paths, raw)?;
            serde_json::from_value(value).map_err(|error| {
                AuError::code("E_DAEMON", format!("invalid action reply: {error}"))
            })
        }
        _ => actions::execute(cli),
    }
}

fn daemon_command(cli: &Cli) -> Result<ActionResult> {
    let paths = AppPaths::discover()?;
    match cli.args.first().map(String::as_str).unwrap_or("status") {
        "serve" => {
            let mut config = android_use::config::load(&paths)?;
            let mut runtime = actions::DaemonRuntime::default();
            daemon::serve(&paths, |argv| {
                let mut nested = Cli::parse(argv)?;
                nested.daemon_child = true;
                serde_json::to_value(actions::execute_daemon(
                    &nested,
                    &paths,
                    &mut config,
                    &mut runtime,
                )?)
                .map_err(Into::into)
            })?;
            Ok(ActionResult {
                brief: Brief::Ok,
                data: serde_json::json!({"stopped":true}),
            })
        }
        "start" => {
            daemon::ensure_started(&paths)?;
            Ok(ActionResult {
                brief: Brief::Ok,
                data: daemon::status(&paths)?,
            })
        }
        "stop" => {
            daemon::stop(&paths)?;
            Ok(ActionResult {
                brief: Brief::Ok,
                data: serde_json::json!({"stopped":true}),
            })
        }
        "status" => Ok(ActionResult {
            brief: Brief::Ok,
            data: daemon::status(&paths)?,
        }),
        "ping" => Ok(ActionResult {
            brief: Brief::Ok,
            data: daemon::hello()?,
        }),
        operation => Err(AuError::code(
            "E_ARGS",
            format!("unknown daemon operation {operation}"),
        )),
    }
}

use std::env;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use android_use::protocol::{read_native_response, write_native_request, Request, RequestBody};
use android_use::{MAX_PROTOCOL_FRAME, PROTOCOL_VERSION};
use serde_json::json;

const PIPE_NAME: &str = r"\\.\pipe\codex-android-use-v1";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.len() < 3 || args[2] != "--" {
        return Err("usage: au-bench-ipc SAMPLES WARMUP -- ARGV...".into());
    }
    let samples: usize = args[0].parse()?;
    let warmup: usize = args[1].parse()?;
    if !(1..=10_000).contains(&samples) || warmup > 1_000 {
        return Err("samples/warmup are outside bounds".into());
    }
    let argv = args[3..].to_vec();
    if argv.is_empty() {
        return Err("ARGV must not be empty".into());
    }

    let mut pipe = open_pipe()?;
    for index in 0..warmup {
        let response = request_on_pipe(&mut pipe, &argv, index as u64)?;
        if !response.ok {
            return Err("warmup request returned an error".into());
        }
    }

    let mut values = Vec::with_capacity(samples);
    for index in 0..samples {
        let started = Instant::now();
        let response = request_on_pipe(&mut pipe, &argv, (index + warmup) as u64)?;
        let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
        if !response.ok {
            return Err("sample request returned an error".into());
        }
        values.push(elapsed);
    }
    let p50 = percentile(&values, 0.50);
    let p95 = percentile(&values, 0.95);
    println!(
        "{}",
        serde_json::to_string(&json!({
            "samples": samples,
            "warmup": warmup,
            "transport": "native-au2-rust-client",
            "p50_ms": round(p50),
            "p95_ms": round(p95),
            "min_ms": round(values.iter().copied().fold(f64::INFINITY, f64::min)),
            "max_ms": round(values.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
            "values_ms": values.iter().map(|value| round(*value)).collect::<Vec<_>>(),
            "max_frame_bytes": MAX_PROTOCOL_FRAME,
        }))?
    );
    Ok(())
}

fn open_pipe() -> Result<std::fs::File, Box<dyn std::error::Error>> {
    let mut last_error = None;
    for _ in 0..20 {
        match OpenOptions::new().read(true).write(true).open(PIPE_NAME) {
            Ok(pipe) => return Ok(pipe),
            Err(error)
                if error.kind() == ErrorKind::NotFound
                    || error.kind() == ErrorKind::WouldBlock
                    || error.raw_os_error() == Some(231) =>
            {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(last_error
        .map(|error| error.into())
        .unwrap_or_else(|| "named pipe did not become available".into()))
}

fn request_on_pipe(
    pipe: &mut std::fs::File,
    argv: &[String],
    nonce: u64,
) -> Result<android_use::protocol::Response, Box<dyn std::error::Error>> {
    let request = Request {
        version: PROTOCOL_VERSION,
        id: request_id(nonce),
        body: RequestBody::Execute {
            argv: argv.to_vec(),
        },
    };
    write_native_request(pipe, &request)?;
    Ok(read_native_response(pipe)?)
}

fn request_id(nonce: u64) -> u64 {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    time ^ nonce.rotate_left(17)
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    let mut ordered = values.to_vec();
    ordered.sort_by(|left, right| left.total_cmp(right));
    let rank = (ordered.len() - 1) as f64 * percentile;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return ordered[lower];
    }
    ordered[lower] + (ordered[upper] - ordered[lower]) * (rank - lower as f64)
}

fn round(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

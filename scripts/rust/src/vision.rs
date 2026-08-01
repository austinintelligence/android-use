use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use png::{BitDepth, ColorType, Decoder, Encoder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::adb::Adb;
use crate::config::{atomic_write, AppPaths};
use crate::error::{AuError, Result};
use crate::files::{reserve_output, screenshot, Artifact};
use crate::helper;

const DEFAULT_DIFF_THRESHOLD: u8 = 8;
const MAX_REGION_HANDLES: usize = 32;

#[derive(Clone, Debug)]
struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RegionHandle {
    handle: String,
    hash: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

pub fn execute(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    args: &[String],
    output: Option<&Path>,
    force: bool,
) -> Result<Value> {
    let operation = args.first().map(String::as_str).unwrap_or("inspect");
    let rest = &args[usize::from(!args.is_empty())..];
    match operation {
        "inspect" | "frontier" => inspect(adb, paths, serial),
        "hash" => hash(adb, paths, serial, rest),
        "diff" => diff(adb, paths, serial, rest),
        "crop" => crop(adb, paths, serial, rest, output, force),
        "region" => region(adb, paths, serial, rest),
        "check" => check(adb, paths, serial, rest),
        "clear" => clear(paths),
        _ => Err(AuError::code(
            "E_ARGS",
            format!(
                "unknown vision operation {operation}; use inspect|hash|diff|crop|region|check|clear"
            ),
        )),
    }
}

fn inspect(adb: &Adb, paths: &AppPaths, serial: &str) -> Result<Value> {
    let mut pool = helper::HelperPool::new();
    let result = pool.call_with_timeout(
        adb,
        paths,
        serial,
        "ui.snapshot",
        json!({"args":["--compact","--frontier"]}),
        Duration::from_secs(8),
    );
    pool.shutdown();
    let snapshot = result?;
    let complete = snapshot
        .get("complete")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let generation = snapshot.get("g").cloned().unwrap_or(Value::Null);
    let nodes = snapshot
        .get("n")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    Ok(json!({
        "level":"semantic_frontier",
        "complete":complete,
        "g":generation,
        "nodes":nodes,
        "next":if complete {"semantic"} else {"whole_screen_hash_or_crop"},
        "reason":if complete {"frontier_complete"} else {"frontier_incomplete"}
    }))
}

fn hash(adb: &Adb, paths: &AppPaths, serial: &str, args: &[String]) -> Result<Value> {
    let (frame, captured_path) = if let Some(path) = args.first() {
        (decode_png(Path::new(path))?, None)
    } else {
        let path = capture(adb, paths, serial)?;
        let result = decode_png(&path);
        let _ = fs::remove_file(&path);
        (result?, Some(path))
    };
    Ok(json!({
        "level":"whole_screen_hash",
        "hash":frame_hash(&frame),
        "width":frame.width,
        "height":frame.height,
        "captured":captured_path.is_some()
    }))
}

fn diff(adb: &Adb, paths: &AppPaths, serial: &str, args: &[String]) -> Result<Value> {
    let base_path = args
        .first()
        .ok_or_else(|| AuError::code("E_ARGS", "vision diff BASE_PNG [THRESHOLD]"))?;
    let threshold = args
        .get(1)
        .map(|value| value.parse::<u8>())
        .transpose()
        .map_err(|_| AuError::code("E_ARGS", "threshold must be 0..255"))?
        .unwrap_or(DEFAULT_DIFF_THRESHOLD);
    let base = decode_png(Path::new(base_path))?;
    let current_path = capture(adb, paths, serial)?;
    let current_result = decode_png(&current_path);
    let _ = fs::remove_file(&current_path);
    let current = current_result?;
    if base.width != current.width || base.height != current.height {
        return Err(AuError::code(
            "E_VISION",
            "base and current screenshots have different dimensions",
        ));
    }
    let changed = changed_region(&base, &current, threshold);
    Ok(json!({
        "level":"changed_region",
        "base_hash":frame_hash(&base),
        "current_hash":frame_hash(&current),
        "threshold":threshold,
        "changed":changed.changed,
        "pixels":changed.pixels,
        "ratio":changed.ratio,
        "region":changed.region
    }))
}

fn crop(
    adb: &Adb,
    paths: &AppPaths,
    serial: &str,
    args: &[String],
    output: Option<&Path>,
    force: bool,
) -> Result<Value> {
    let (frame, temporary) = current_frame(adb, paths, serial)?;
    let result = (|| {
        let bounds = bounds_from_args(args, frame.width, frame.height, "vision crop X Y W H")?;
        let rgba = crop_rgba(&frame, bounds)?;
        let destination = reserve_output(paths, output, "vision-crop", "png", force)?;
        write_png(&destination, bounds.2, bounds.3, &rgba)?;
        let artifact = artifact(&destination)?;
        let hash = frame_hash(&frame);
        let handle = register_region(paths, &hash, bounds)?;
        Ok(json!({
            "level":"exact_crop",
            "region":handle,
            "hash":hash,
            "x":bounds.0,
            "y":bounds.1,
            "width":bounds.2,
            "height":bounds.3,
            "path":artifact.path,
            "bytes":artifact.bytes,
            "sha256":artifact.sha256
        }))
    })();
    let _ = fs::remove_file(temporary);
    result
}

fn region(adb: &Adb, paths: &AppPaths, serial: &str, args: &[String]) -> Result<Value> {
    let (frame, temporary) = current_frame(adb, paths, serial)?;
    let result = bounds_from_args(args, frame.width, frame.height, "vision region X Y W H")
        .and_then(|bounds| {
            let hash = frame_hash(&frame);
            let handle = register_region(paths, &hash, bounds)?;
            Ok(json!({
                "level":"region_handle",
                "region":handle,
                "hash":hash,
                "x":bounds.0,
                "y":bounds.1,
                "width":bounds.2,
                "height":bounds.3
            }))
        });
    let _ = fs::remove_file(temporary);
    result
}

fn check(adb: &Adb, paths: &AppPaths, serial: &str, args: &[String]) -> Result<Value> {
    let handle = args
        .first()
        .ok_or_else(|| AuError::code("E_ARGS", "vision check REGION_HANDLE"))?;
    let regions = load_regions(paths)?;
    let stored = regions
        .iter()
        .find(|region| region.handle == *handle)
        .ok_or_else(|| AuError::code("E_STALE", "unknown visual region handle"))?;
    let (frame, temporary) = current_frame(adb, paths, serial)?;
    let current_hash = frame_hash(&frame);
    let _ = fs::remove_file(temporary);
    if current_hash != stored.hash {
        return Err(AuError::code(
            "E_STALE",
            "visual region handle is stale; refresh the visual region",
        ));
    }
    Ok(json!({
        "valid":true,
        "region":stored.handle,
        "hash":current_hash,
        "x":stored.x,
        "y":stored.y,
        "width":stored.width,
        "height":stored.height
    }))
}

fn clear(paths: &AppPaths) -> Result<Value> {
    let path = region_path(paths);
    let existed = path.exists();
    if existed {
        fs::remove_file(path)?;
    }
    Ok(json!({"cleared":existed}))
}

fn capture(adb: &Adb, paths: &AppPaths, serial: &str) -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuError::code("E_TIME", "system clock before epoch"))?
        .as_nanos();
    let path = paths.state.join(format!("vision-capture-{nonce}.png"));
    match screenshot(adb, serial, path.clone()) {
        Ok(_) => Ok(path),
        Err(error) => {
            let _ = fs::remove_file(&path);
            Err(error)
        }
    }
}

fn current_frame(adb: &Adb, paths: &AppPaths, serial: &str) -> Result<(Frame, PathBuf)> {
    let path = capture(adb, paths, serial)?;
    match decode_png(&path) {
        Ok(frame) => Ok((frame, path)),
        Err(error) => {
            let _ = fs::remove_file(&path);
            Err(error)
        }
    }
}

fn decode_png(path: &Path) -> Result<Frame> {
    let file = File::open(path)?;
    let decoder = Decoder::new(file);
    let mut reader = decoder
        .read_info()
        .map_err(|error| AuError::code("E_VISION", format!("decode PNG header: {error}")))?;
    let mut bytes = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut bytes)
        .map_err(|error| AuError::code("E_VISION", format!("decode PNG pixels: {error}")))?;
    if info.bit_depth != BitDepth::Eight {
        return Err(AuError::code(
            "E_VISION",
            "only 8-bit PNG screenshots are supported",
        ));
    }
    let source = &bytes[..info.buffer_size()];
    let rgba = match info.color_type {
        ColorType::Rgba => source.to_vec(),
        ColorType::Rgb => source
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        ColorType::Grayscale => source
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect(),
        ColorType::GrayscaleAlpha => source
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        ColorType::Indexed => {
            return Err(AuError::code(
                "E_VISION",
                "indexed PNG screenshots are unsupported",
            ));
        }
    };
    Ok(Frame {
        width: info.width,
        height: info.height,
        rgba,
    })
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let file = File::create(path)?;
    let mut encoder = Encoder::new(file, width, height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| AuError::code("E_VISION", format!("write PNG header: {error}")))?;
    writer
        .write_image_data(rgba)
        .map_err(|error| AuError::code("E_VISION", format!("write PNG pixels: {error}")))?;
    Ok(())
}

fn crop_rgba(frame: &Frame, bounds: (u32, u32, u32, u32)) -> Result<Vec<u8>> {
    let (x, y, width, height) = bounds;
    let mut cropped = Vec::with_capacity((width * height * 4) as usize);
    for row in y..y + height {
        let start = ((row * frame.width + x) * 4) as usize;
        let end = start + (width * 4) as usize;
        cropped.extend_from_slice(&frame.rgba[start..end]);
    }
    Ok(cropped)
}

fn bounds_from_args(
    args: &[String],
    max_width: u32,
    max_height: u32,
    usage: &str,
) -> Result<(u32, u32, u32, u32)> {
    if args.len() != 4 {
        return Err(AuError::code("E_ARGS", usage));
    }
    let x = coordinate(&args[0], max_width)?;
    let y = coordinate(&args[1], max_height)?;
    let width = coordinate(&args[2], max_width)?;
    let height = coordinate(&args[3], max_height)?;
    if width == 0
        || height == 0
        || x.saturating_add(width) > max_width
        || y.saturating_add(height) > max_height
    {
        return Err(AuError::code(
            "E_ARGS",
            "visual region must be non-empty and inside the screenshot",
        ));
    }
    Ok((x, y, width, height))
}

fn coordinate(value: &str, extent: u32) -> Result<u32> {
    if let Some(percent) = value.strip_suffix('%') {
        let value = percent
            .parse::<f64>()
            .map_err(|_| AuError::code("E_ARGS", "visual percentage is invalid"))?;
        if !(0.0..=100.0).contains(&value) {
            return Err(AuError::code("E_ARGS", "visual percentage must be 0..100"));
        }
        return Ok((value * f64::from(extent) / 100.0).round() as u32);
    }
    value
        .parse::<u32>()
        .map_err(|_| AuError::code("E_ARGS", "visual coordinate must be pixels or percentage"))
}

fn frame_hash(frame: &Frame) -> String {
    let mut hasher = Sha256::new();
    hasher.update(frame.width.to_le_bytes());
    hasher.update(frame.height.to_le_bytes());
    hasher.update(&frame.rgba);
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct ChangedRegion {
    changed: bool,
    pixels: u64,
    ratio: f64,
    region: Value,
}

fn changed_region(base: &Frame, current: &Frame, threshold: u8) -> ChangedRegion {
    let mut min_x = current.width;
    let mut min_y = current.height;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut pixels = 0u64;
    for y in 0..current.height {
        for x in 0..current.width {
            let index = ((y * current.width + x) * 4) as usize;
            let different = (0..4).any(|channel| {
                base.rgba[index + channel].abs_diff(current.rgba[index + channel]) > threshold
            });
            if different {
                pixels += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    let changed = pixels > 0;
    let region = if changed {
        json!({
            "x":min_x,
            "y":min_y,
            "width":max_x - min_x + 1,
            "height":max_y - min_y + 1
        })
    } else {
        Value::Null
    };
    ChangedRegion {
        changed,
        pixels,
        ratio: pixels as f64 / f64::from(current.width.saturating_mul(current.height)),
        region,
    }
}

fn artifact(path: &Path) -> Result<Artifact> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(Artifact {
        path: path.display().to_string(),
        bytes: bytes.len() as u64,
        sha256: hex(&hasher.finalize()),
    })
}

fn region_path(paths: &AppPaths) -> PathBuf {
    paths.state.join("vision-regions.json")
}

fn load_regions(paths: &AppPaths) -> Result<Vec<RegionHandle>> {
    let path = region_path(paths);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| AuError::code("E_VISION", format!("invalid visual region state: {error}")))
}

fn register_region(paths: &AppPaths, hash: &str, bounds: (u32, u32, u32, u32)) -> Result<String> {
    let mut regions = load_regions(paths)?;
    let handle = format!("r{}", &hash[..12]);
    regions.retain(|region| region.handle != handle);
    regions.push(RegionHandle {
        handle: handle.clone(),
        hash: hash.into(),
        x: bounds.0,
        y: bounds.1,
        width: bounds.2,
        height: bounds.3,
    });
    if regions.len() > MAX_REGION_HANDLES {
        let remove = regions.len() - MAX_REGION_HANDLES;
        regions.drain(..remove);
    }
    atomic_write(&region_path(paths), &serde_json::to_vec(&regions)?)?;
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::{changed_region, coordinate, Frame};

    fn frame(width: u32, height: u32) -> Frame {
        Frame {
            width,
            height,
            rgba: vec![0; (width * height * 4) as usize],
        }
    }

    #[test]
    fn changed_region_is_bounded_to_changed_pixels() {
        let base = frame(4, 3);
        let mut current = frame(4, 3);
        let index = ((4 + 2) * 4) as usize;
        current.rgba[index] = 255;
        let result = changed_region(&base, &current, 8);
        assert!(result.changed);
        assert_eq!(result.pixels, 1);
        assert_eq!(result.region["x"], 2);
        assert_eq!(result.region["y"], 1);
        assert_eq!(result.region["width"], 1);
        assert_eq!(result.region["height"], 1);
    }

    #[test]
    fn visual_coordinates_accept_pixels_and_percentages() {
        assert_eq!(coordinate("50%", 800).expect("percent"), 400);
        assert_eq!(coordinate("12", 800).expect("pixels"), 12);
    }
}

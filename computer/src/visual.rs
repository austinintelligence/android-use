use crate::api::{Code, Error, Result};
use png::{BitDepth, ColorType, Decoder, Encoder};
use serde_json::{json, Value};
use std::io::Cursor;

const MAX_PIXELS: u64 = 8_000_000;
const MAX_BYTES: usize = 16 * 1024 * 1024;

pub enum Op {
    Crop { bytes: Vec<u8>, x: u32, y: u32, w: u32, h: u32 },
}

pub fn hash(bytes: &[u8]) -> Result<Value> {
    let image = decode(bytes)?;
    let mut bits = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            bits <<= 1;
            bits |= u64::from(gray(&image, x * 9 / 8, y * 8 / 8) > gray(&image, (x + 1) * 9 / 8, y * 8 / 8));
        }
    }
    Ok(json!({"w":image.w,"h":image.h,"hash":format!("{bits:016x}")}))
}

pub fn diff(a: &[u8], b: &[u8]) -> Result<Value> {
    let left = decode(a)?;
    let right = decode(b)?;
    if left.w != right.w || left.h != right.h {
        return Ok(json!({"changed":1,"ratio":1.0,"w":[left.w,right.w],"h":[left.h,right.h]}));
    }
    let step = ((left.w as u64 * left.h as u64) / 100_000).max(1) as usize;
    let mut changed = 0usize;
    let mut total = 0usize;
    for i in (0..(left.w as usize * left.h as usize)).step_by(step) {
        let p = i * 4;
        let delta = (left.pixels[p] as i16 - right.pixels[p] as i16).unsigned_abs() as u32
            + (left.pixels[p + 1] as i16 - right.pixels[p + 1] as i16).unsigned_abs() as u32
            + (left.pixels[p + 2] as i16 - right.pixels[p + 2] as i16).unsigned_abs() as u32;
        if delta > 24 {
            changed += 1;
        }
        total += 1;
    }
    let ratio = changed as f64 / total.max(1) as f64;
    Ok(json!({"changed":u8::from(ratio>0.01),"ratio":ratio,"w":left.w,"h":left.h}))
}

pub fn crop(bytes: Vec<u8>, x: u32, y: u32, w: u32, h: u32) -> Result<Vec<u8>> {
    let image = decode(&bytes)?;
    if w == 0 || h == 0 || x >= image.w || y >= image.h || x.saturating_add(w) > image.w || y.saturating_add(h) > image.h {
        return Err(Error::new(Code::Args, "crop rectangle is outside the image"));
    }
    let mut out = Vec::with_capacity((w as usize).saturating_mul(h as usize).saturating_mul(4));
    for row in 0..h {
        let start = ((y + row) as usize * image.w as usize + x as usize) * 4;
        out.extend_from_slice(&image.pixels[start..start + w as usize * 4]);
    }
    encode(w, h, &out)
}

struct Image {
    w: u32,
    h: u32,
    pixels: Vec<u8>,
}

fn decode(bytes: &[u8]) -> Result<Image> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::new(Code::Bounds, "image exceeds 16 MiB"));
    }
    let mut decoder = Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png::Limits { bytes: MAX_BYTES });
    let mut reader = decoder.read_info().map_err(|_| Error::new(Code::Protocol, "PNG header is invalid"))?;
    let (width, height) = (reader.info().width, reader.info().height);
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels == 0 || pixels > MAX_PIXELS {
        return Err(Error::new(Code::Bounds, "image dimensions are too large"));
    }
    if reader.output_color_type().0 == ColorType::Indexed || reader.output_color_type().1 != BitDepth::Eight {
        return Err(Error::new(Code::Unsupported, "only 8-bit RGB or RGBA PNGs are supported"));
    }
    let mut raw = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut raw).map_err(|_| Error::new(Code::Protocol, "PNG pixels are invalid"))?;
    let mut rgba = Vec::with_capacity(pixels as usize * 4);
    match frame.color_type {
        ColorType::Rgba => rgba.extend_from_slice(&raw[..frame.buffer_size()]),
        ColorType::Rgb => {
            for p in raw[..frame.buffer_size()].chunks_exact(3) {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        ColorType::Grayscale => {
            for &v in &raw[..frame.buffer_size()] {
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        ColorType::GrayscaleAlpha => {
            for p in raw[..frame.buffer_size()].chunks_exact(2) {
                rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
            }
        }
        ColorType::Indexed => return Err(Error::new(Code::Unsupported, "indexed PNGs are unsupported")),
    }
    Ok(Image { w: width, h: height, pixels: rgba })
}

fn encode(w: u32, h: u32, pixels: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = Encoder::new(&mut out, w, h);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|_| Error::new(Code::Io, "PNG encoder failed"))?;
    writer.write_image_data(pixels).map_err(|_| Error::new(Code::Io, "PNG encoder failed"))?;
    drop(writer);
    if out.len() > crate::api::MAX_FRAME {
        return Err(Error::new(Code::Bounds, "cropped artifact exceeds 1 MiB"));
    }
    Ok(out)
}

fn gray(image: &Image, x: u32, y: u32) -> u8 {
    let x = x.min(image.w - 1);
    let y = y.min(image.h - 1);
    let p = ((y * image.w + x) * 4) as usize;
    ((u16::from(image.pixels[p]) * 3 + u16::from(image.pixels[p + 1]) * 6 + u16::from(image.pixels[p + 2])) / 10) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_hash_and_diff_are_bounded() {
        let bytes = encode(2, 2, &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]).unwrap();
        assert_eq!(hash(&bytes).unwrap()["w"], 2);
        assert_eq!(diff(&bytes, &bytes).unwrap()["changed"], 0);
        assert!(crop(bytes, 0, 0, 1, 1).unwrap().len() > 20);
    }
}

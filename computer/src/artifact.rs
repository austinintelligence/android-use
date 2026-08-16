use crate::{
    api::{Code, Error, Range, Result, MAX_INLINE},
    device::{atomic, Paths},
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
};

pub const MAX_ARTIFACT: usize = 16 * 1024 * 1024;

pub struct Artifacts {
    paths: Paths,
}

impl Artifacts {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }
    pub fn put(&self, bytes: &[u8]) -> Result<Box<str>> {
        if bytes.len() > MAX_ARTIFACT {
            return Err(Error::new(Code::Bounds, "artifact exceeds 16 MiB"));
        }
        let digest = Sha256::digest(bytes);
        let id = format!("h{}", digest[..12].iter().map(|b| format!("{b:02x}")).collect::<String>());
        let path = self.paths.artifacts.join(&id);
        if !path.exists() {
            atomic(&path, bytes)?;
        }
        Ok(id.into_boxed_str())
    }
    pub fn read(&self, id: &str, range: Option<Range>) -> Result<Value> {
        if !valid(id) || !id.starts_with('h') {
            return Err(Error::new(Code::Artifact, "unknown host artifact"));
        }
        let path = self.paths.artifacts.join(id);
        let meta = fs::symlink_metadata(&path).map_err(|_| Error::new(Code::Artifact, "artifact not found"))?;
        if !meta.file_type().is_file() || meta.file_type().is_symlink() {
            return Err(Error::new(Code::Artifact, "artifact is not a regular file"));
        }
        let size = usize::try_from(meta.len()).map_err(|_| Error::new(Code::Bounds, "artifact is too large"))?;
        let r = range.unwrap_or(Range { start: 0, end: size.min(MAX_INLINE) as u64 }).bounded(size as u64)?;
        let start = r.start as u64;
        let end = r.end;
        let mut bytes = vec![0; r.len()];
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut bytes)?;
        Ok(json!({"id":id,"size":size,"start":start,"data":STANDARD.encode(bytes),"more":(end<size) as u8}))
    }
    pub fn bytes(&self, id: &str) -> Result<Vec<u8>> {
        if !valid(id) || !id.starts_with('h') {
            return Err(Error::new(Code::Artifact, "unknown host artifact"));
        }
        let path = self.paths.artifacts.join(id);
        let meta = fs::symlink_metadata(&path).map_err(|_| Error::new(Code::Artifact, "artifact not found"))?;
        if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() > MAX_ARTIFACT as u64 {
            return Err(Error::new(Code::Bounds, "artifact is too large or not a regular file"));
        }
        Ok(fs::read(path)?)
    }
}

pub fn valid(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let a = Artifacts::new(Paths::at(d.path().to_path_buf()).unwrap());
        let id = a.put(b"abc").unwrap();
        assert_eq!(a.read(&id, None).unwrap()["size"], 3);
    }
    #[test]
    fn rejects_paths() {
        assert!(!valid("../x"));
    }

    #[test]
    fn media_sized_artifacts_are_read_in_bounded_chunks() {
        let d = tempfile::tempdir().unwrap();
        let a = Artifacts::new(Paths::at(d.path().to_path_buf()).unwrap());
        let id = a.put(&vec![7; crate::api::MAX_FRAME + 1]).unwrap();
        let first = a.read(&id, None).unwrap();
        assert_eq!(first["size"], crate::api::MAX_FRAME + 1);
        assert_eq!(first["more"], 1);
        assert_eq!(a.read(&id, Some(Range { start: crate::api::MAX_INLINE as u64, end: crate::api::MAX_INLINE as u64 + 1 })).unwrap()["start"], crate::api::MAX_INLINE);
        assert_eq!(a.bytes(&id).unwrap().len(), crate::api::MAX_FRAME + 1);
    }
}

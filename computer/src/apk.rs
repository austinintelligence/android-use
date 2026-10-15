//! Reads the signer certificates from an APK's v2/v3 signing block, so a release `au` can refuse an
//! untrusted helper (v2 `0x7109871a`, v3 `0xf05368c0`, v3.1 `0x1b93ad61`) before `adb install`. Android still verifies the signature itself when installing.
//! Signature-scheme blocks with other IDs are ignored: Android verifies only v2, v3 and v3.1.
use sha2::{Digest, Sha256};

const PINNED_SIGNER: &str = include_str!("../../device/release-signer.sha256");

fn le(b: &[u8], at: usize, n: usize) -> Option<usize> {
    let v = b.get(at..at.checked_add(n)?)?.iter().rev().fold(0u64, |a, x| a << 8 | u64::from(*x));
    usize::try_from(v).ok()
}
/// A 4-byte length-prefixed slice at `*at`, advancing past it.
fn prefixed<'a>(b: &'a [u8], at: &mut usize) -> Option<&'a [u8]> {
    let start = at.checked_add(4)?;
    let part = b.get(start..start.checked_add(le(b, *at, 4)?)?)?;
    *at = start + part.len();
    Some(part)
}
/// SHA-256 of each signer's first certificate in one v2/v3 block value.
fn signer_hashes(block: &[u8], out: &mut Vec<String>) -> Option<()> {
    let (mut at, mut next) = (0, 0);
    let signers = prefixed(block, &mut at)?;
    while next < signers.len() {
        let (mut s, mut d, mut c) = (0, 0, 0);
        let data = prefixed(prefixed(signers, &mut next)?, &mut s)?;
        prefixed(data, &mut d)?;
        let cert = prefixed(prefixed(data, &mut d)?, &mut c)?;
        out.push(Sha256::digest(cert).iter().map(|b| format!("{b:02x}")).collect());
    }
    Some(())
}
/// True only when the APK has a v2/v3 signature and every signer certificate is the pinned release one.
pub fn trusted(apk: &[u8]) -> bool {
    trusted_by(apk, PINNED_SIGNER.trim())
}
fn trusted_by(apk: &[u8], pinned: &str) -> bool {
    let mut hashes = Vec::new();
    let found = (|| {
        // The end-of-central-directory record must close the file exactly, as Android requires.
        let eocd = apk.windows(4).rposition(|w| w == [0x50, 0x4b, 5, 6])?;
        if eocd + 22 + le(apk, eocd + 20, 2)? != apk.len() {
            return None;
        }
        let dir = le(apk, eocd + 16, 4)?;
        let end = dir.checked_sub(24)?;
        if apk.get(dir.checked_sub(16)?..dir)? != b"APK Sig Block 42" {
            return None;
        }
        let mut at = dir.checked_sub(le(apk, end, 8)?)?;
        while at < end {
            let len = le(apk, at, 8)?;
            let value = apk.get(at + 12..at.checked_add(8)?.checked_add(len)?)?;
            if matches!(le(apk, at + 8, 4)?, 0x7109_871a | 0xf053_68c0 | 0x1b93_ad61) {
                signer_hashes(value, &mut hashes)?;
            }
            at = at.checked_add(8)?.checked_add(len)?;
        }
        Some(())
    })();
    let pinned = pinned.to_ascii_lowercase();
    found.is_some() && !hashes.is_empty() && hashes.iter().all(|h| *h == pinned)
}

#[cfg(test)]
#[path = "apk_tests.rs"]
mod tests;

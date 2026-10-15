use super::*;

fn prefixed_bytes(parts: &[&[u8]]) -> Vec<u8> {
    parts.iter().flat_map(|p| (p.len() as u32).to_le_bytes().into_iter().chain(p.iter().copied())).collect()
}

/// A minimal APK: one signing block pair holding `cert`, then an empty central directory.
fn apk_signed_by(cert: &[u8]) -> Vec<u8> {
    let certs = prefixed_bytes(&[cert]);
    let data = prefixed_bytes(&[b"", &certs]);
    let signer = prefixed_bytes(&[&data]);
    let value = prefixed_bytes(&[&prefixed_bytes(&[&signer])]);
    let mut pair = ((value.len() + 4) as u64).to_le_bytes().to_vec();
    pair.extend(0x7109_871au32.to_le_bytes());
    pair.extend(&value);
    let size = (pair.len() + 24) as u64;
    let mut apk = b"PK\x03\x04".to_vec();
    apk.extend(size.to_le_bytes());
    apk.extend(&pair);
    apk.extend(size.to_le_bytes());
    apk.extend(b"APK Sig Block 42");
    let dir = apk.len() as u32;
    apk.extend([0x50, 0x4b, 5, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    apk.extend(dir.to_le_bytes());
    apk.extend([0, 0]);
    apk
}

#[test]
fn only_an_apk_signed_by_the_pinned_certificate_is_trusted() {
    // A real certificate is not needed: the hash of this stand-in is what the test pins.
    let cert = b"stand-in certificate";
    let hash: String = Sha256::digest(cert).iter().map(|b| format!("{b:02x}")).collect();
    let apk = apk_signed_by(cert);
    assert!(trusted_by(&apk, &hash.to_uppercase()));
    assert!(!trusted_by(&apk, &"0".repeat(64)));
    assert!(!trusted(&apk), "a certificate other than the pinned one is refused");
    assert!(!trusted(b"PK\x03\x04 not signed"));
    assert!(!trusted_by(&[apk.as_slice(), b"x"].concat(), &hash), "bytes after the central directory record are refused");
}

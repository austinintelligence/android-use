use super::*;

fn prefixed_bytes(parts: &[&[u8]]) -> Vec<u8> {
    parts.iter().flat_map(|p| (p.len() as u32).to_le_bytes().into_iter().chain(p.iter().copied())).collect()
}

/// A minimal APK: one signing-block pair per `(id, cert)`, then an empty central directory.
fn apk_with(blocks: &[(u32, &[u8])]) -> Vec<u8> {
    let pairs: Vec<u8> = blocks
        .iter()
        .flat_map(|(id, cert)| {
            let data = prefixed_bytes(&[b"", &prefixed_bytes(&[cert])]);
            let signers = prefixed_bytes(&[&prefixed_bytes(&[&data])]);
            let value = prefixed_bytes(&[&signers]);
            let mut pair = ((value.len() + 4) as u64).to_le_bytes().to_vec();
            pair.extend(id.to_le_bytes());
            pair.extend(&value);
            pair
        })
        .collect();
    let size = (pairs.len() + 24) as u64;
    let mut apk = b"PK\x03\x04".to_vec();
    apk.extend(size.to_le_bytes());
    apk.extend(&pairs);
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
    let apk = apk_with(&[(0x7109_871a, cert)]);
    assert!(trusted_by(&apk, &hash.to_uppercase()));
    assert!(!trusted_by(&apk, &"0".repeat(64)));
    assert!(!trusted(&apk), "a certificate other than the pinned one is refused");
    assert!(!trusted(b"PK\x03\x04 not signed"));
    assert!(!trusted_by(&[apk.as_slice(), b"x"].concat(), &hash), "bytes after the central directory record are refused");
}

#[test]
fn a_real_apksigner_signed_apk_is_read_and_a_different_pin_is_refused() {
    // Signed with a throwaway test key (v2 and v3, minSdk 26) by Google's apksig library.
    let apk = include_bytes!("../tests/fixtures/signed.apk");
    assert!(trusted_by(apk, "07dfcef9876ba30937fb90ba658ea372610503a2ed235926f90777c645387e3b"));
    assert!(!trusted_by(apk, &"0".repeat(64)));
    assert!(!trusted(apk), "the test key is not the release key");
}

#[test]
fn a_v3_block_with_another_certificate_defeats_a_pinned_v2_block() {
    let hash: String = Sha256::digest(b"pinned").iter().map(|b| format!("{b:02x}")).collect();
    assert!(trusted_by(&apk_with(&[(0x7109_871a, b"pinned"), (0xf053_68c0, b"pinned")]), &hash));
    assert!(!trusted_by(&apk_with(&[(0x7109_871a, b"pinned"), (0xf053_68c0, b"attacker")]), &hash));
    assert!(!trusted_by(&apk_with(&[(0x1b93_ad61, b"attacker")]), &hash));
}

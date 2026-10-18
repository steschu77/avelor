//! SHA-256: BCrypt on Windows, GLib's checksum on Linux.

// ----------------------------------------------------------------------------
#[cfg(target_os = "windows")]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    use windows::Win32::Security::Cryptography::{
        BCRYPT_ALG_HANDLE, BCRYPT_HASH_HANDLE, BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS,
        BCRYPT_SHA256_ALGORITHM, BCryptCloseAlgorithmProvider, BCryptCreateHash, BCryptDestroyHash,
        BCryptFinishHash, BCryptHashData, BCryptOpenAlgorithmProvider,
    };
    use windows::core::PCWSTR;

    unsafe {
        let mut alg = BCRYPT_ALG_HANDLE::default();
        BCryptOpenAlgorithmProvider(
            &mut alg,
            BCRYPT_SHA256_ALGORITHM,
            PCWSTR::null(),
            BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0),
        )
        .ok()
        .expect("BCrypt SHA-256 provider unavailable");
        let mut hash = BCRYPT_HASH_HANDLE::default();
        BCryptCreateHash(alg, &mut hash, None, None, 0)
            .ok()
            .expect("BCryptCreateHash failed");
        BCryptHashData(hash, data, 0)
            .ok()
            .expect("BCryptHashData failed");
        let mut out = [0u8; 32];
        BCryptFinishHash(hash, &mut out, 0)
            .ok()
            .expect("BCryptFinishHash failed");
        let _ = BCryptDestroyHash(hash);
        let _ = BCryptCloseAlgorithmProvider(alg, 0);
        out
    }
}

// ----------------------------------------------------------------------------
#[cfg(target_os = "linux")]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut checksum = glib::Checksum::new(glib::ChecksumType::Sha256)
        .expect("GLib's SHA-256 checksum unavailable");
    checksum.update(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&checksum.digest());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn sha256_known_vectors() {
        // FIPS 180-2 test vector for "abc".
        let abc = sha256(b"abc");
        assert_eq!(
            abc[..4],
            [0xba, 0x78, 0x16, 0xbf],
            "SHA-256(\"abc\") must start ba7816bf"
        );
        let empty = sha256(b"");
        assert_eq!(empty[..4], [0xe3, 0xb0, 0xc4, 0x42]);
    }
}

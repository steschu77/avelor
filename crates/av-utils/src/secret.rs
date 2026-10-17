//! Protection of secrets at rest (OAuth refresh tokens, API keys): the
//! ciphertext can only be decrypted by the same OS user (DPAPI). On Linux
//! the bytes stay as they are; the file holding them is the user's alone
//! (`fs::write_private`).

#[cfg(target_os = "windows")]
use crate::error::Error;
use crate::error::Result;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{HLOCAL, LocalFree};
#[cfg(target_os = "windows")]
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
#[cfg(target_os = "windows")]
use windows::core::PCWSTR;

// ----------------------------------------------------------------------------
#[cfg(target_os = "windows")]
fn input_blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    }
}

// ----------------------------------------------------------------------------
/// Copy a DPAPI-allocated output blob into a Vec and free it.
#[cfg(target_os = "windows")]
unsafe fn take_output_blob(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    unsafe {
        let bytes = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(blob.pbData as *mut _)));
        bytes
    }
}

// ----------------------------------------------------------------------------
/// Encrypt `plain` for the current OS user.
#[cfg(target_os = "windows")]
pub fn protect(plain: &[u8]) -> Result<Vec<u8>> {
    let input = input_blob(plain);
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(Error::from)?;
        Ok(take_output_blob(output))
    }
}

// ----------------------------------------------------------------------------
/// Decrypt what `protect` produced (same OS user only).
#[cfg(target_os = "windows")]
pub fn unprotect(cipher: &[u8]) -> Result<Vec<u8>> {
    let input = input_blob(cipher);
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(Error::from)?;
        Ok(take_output_blob(output))
    }
}

// ----------------------------------------------------------------------------
/// `plain` as it is.
#[cfg(target_os = "linux")]
pub fn protect(plain: &[u8]) -> Result<Vec<u8>> {
    Ok(plain.to_vec())
}

// ----------------------------------------------------------------------------
/// What `protect` produced.
#[cfg(target_os = "linux")]
pub fn unprotect(cipher: &[u8]) -> Result<Vec<u8>> {
    Ok(cipher.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn protect_round_trips_for_this_user() {
        let secret = b"1//refresh-token-example";
        let cipher = protect(secret).expect("protect");
        #[cfg(target_os = "windows")]
        assert_ne!(cipher, secret);
        assert_eq!(unprotect(&cipher).expect("unprotect"), secret);
    }
}

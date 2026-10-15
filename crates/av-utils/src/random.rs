//! Randomness from the operating system: BCrypt on Windows, `getrandom`
//! on Linux.

// ----------------------------------------------------------------------------
#[cfg(target_os = "windows")]
pub fn os_random(buf: &mut [u8]) {
    use windows::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };
    unsafe {
        BCryptGenRandom(None, buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG)
            .ok()
            .expect("BCryptGenRandom failed - no OS randomness available");
    }
}

// ----------------------------------------------------------------------------
#[cfg(target_os = "linux")]
pub fn os_random(buf: &mut [u8]) {
    let mut filled = 0;
    while filled < buf.len() {
        let rest = &mut buf[filled..];
        let n = unsafe { libc::getrandom(rest.as_mut_ptr().cast(), rest.len(), 0) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            panic!("getrandom failed - no OS randomness available: {err}");
        }
        filled += n as usize;
    }
}

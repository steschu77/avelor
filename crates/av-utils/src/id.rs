//! UUIDv7: 48-bit Unix-ms timestamp, version/variant bits, 74 bits of randomness.
//! Time-ordered, lexicographic order of the canonical string form is chronological order.
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::random::os_random;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(pub [u8; 16]);

// ----------------------------------------------------------------------------
impl Id {
    // ------------------------------------------------------------------------
    pub fn generate() -> Self {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let mut bytes = [0u8; 16];
        bytes[0..6].copy_from_slice(&ms.to_be_bytes()[2..8]);
        os_random(&mut bytes[6..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x70; // version 7
        bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 10
        Id(bytes)
    }

    // ------------------------------------------------------------------------
    /// Parses the canonical `8-4-4-4-12` hex form back into an `Id`.
    pub fn parse(s: &str) -> crate::error::Result<Self> {
        let hex: String = s.chars().filter(|c| *c != '-').collect();
        if hex.len() != 32 {
            return Err(not_an_id(s));
        }
        let mut bytes = [0u8; 16];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let pair = std::str::from_utf8(chunk).map_err(|_| not_an_id(s))?;
            bytes[i] = u8::from_str_radix(pair, 16).map_err(|_| not_an_id(s))?;
        }
        Ok(Id(bytes))
    }

    // ------------------------------------------------------------------------
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    // ------------------------------------------------------------------------
    pub fn as_str(self) -> String {
        let mut out = String::with_capacity(36);
        for (i, b) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                out.push('-');
            }
            out.push_str(&format!("{b:02x}"));
        }
        out
    }

    // ------------------------------------------------------------------------
    pub fn as_timestamp_ms(&self) -> u64 {
        let mut ms_bytes = [0u8; 8];
        ms_bytes[2..8].copy_from_slice(&self.0[0..6]);
        u64::from_be_bytes(ms_bytes)
    }
}

// ----------------------------------------------------------------------------
impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

// ----------------------------------------------------------------------------
impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

// ----------------------------------------------------------------------------
/// The error of a string that isn't an id.
fn not_an_id(s: &str) -> crate::error::Error {
    crate::error::Error::InvalidArgument {
        arg: format!("id {s:?}"),
    }
}

// ----------------------------------------------------------------------------
impl Serialize for Id {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_str())
    }
}

// ----------------------------------------------------------------------------
impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Id::parse(&s).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use crate::id::Id;

    // ------------------------------------------------------------------------
    #[test]
    fn uuidv7_version_and_variant_bits() {
        let id = Id::generate();
        let bytes = id.as_bytes();
        assert_eq!(bytes[6] & 0xf0, 0x70);
        assert_eq!(bytes[8] & 0xc0, 0x80);

        let s = id.as_str();
        assert_eq!(s.len(), 36);

        let chars: Vec<char> = s.chars().collect();
        assert_eq!(chars[8], '-');
        assert_eq!(chars[13], '-');
        assert_eq!(chars[18], '-');
        assert_eq!(chars[23], '-');
        assert_eq!(chars[14], '7', "version nibble must be 7");
        assert!(
            matches!(chars[19], '8' | '9' | 'a' | 'b'),
            "variant bits must be 10xx"
        );
    }
}

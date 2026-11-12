//! A patch's fields: a key absent changes nothing, `null` removes the
//! value, anything else sets it.

use serde::{Deserialize, Deserializer};

// ----------------------------------------------------------------------------
/// `deserialize_with` for an `Option<Option<T>>` with `#[serde(default)]`:
/// absent is `None`, `null` is `Some(None)`.
pub fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

pub mod cancel;
pub mod datetime;
pub mod error;
pub mod fs;
pub mod id;
pub mod logger;
pub mod random;
pub mod text;

// ----------------------------------------------------------------------------
/// The serde skip of a flag written only when it is set
/// (`skip_serializing_if = "is_false"`).
pub fn is_false(value: &bool) -> bool {
    !*value
}

pub mod datetime;
pub mod error;
pub mod fs;
pub mod id;
pub mod logger;
pub mod random;

// ----------------------------------------------------------------------------
/// The serde skip of a flag written only when it is set
/// (`skip_serializing_if = "is_false"`).
pub fn is_false(value: &bool) -> bool {
    !*value
}

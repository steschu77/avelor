pub mod datetime;
pub mod error;
pub mod fs;
pub mod logger;

// ----------------------------------------------------------------------------
/// The serde skip of a flag written only when it is set
/// (`skip_serializing_if = "is_false"`).
pub fn is_false(value: &bool) -> bool {
    !*value
}

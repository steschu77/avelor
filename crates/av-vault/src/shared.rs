//! Small value types shared across several kinds' typed properties: a
//! reference to a contact (by ID or free-text name) and a color (hex or
//! free-text name). See vault.md#shared-types.

use av_utils::id::Id;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    #[default]
    None,
    One(T),
    Many(Vec<T>),
}

// ----------------------------------------------------------------------------
impl<T> OneOrMany<T> {
    /// Whether nothing is given (the key is absent or `null`).
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    pub fn into_vec(self) -> Vec<T> {
        match self {
            Self::None => vec![],
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }

    /// The values, however many there are.
    pub fn as_slice(&self) -> &[T] {
        match self {
            Self::None => &[],
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(values) => values,
        }
    }

    /// `values` in the shortest form: nothing, one value, or a list.
    pub fn from_vec(mut values: Vec<T>) -> Self {
        match values.len() {
            0 => Self::None,
            1 => Self::One(values.remove(0)),
            _ => Self::Many(values),
        }
    }
}

// ----------------------------------------------------------------------------
/// Either a link to a contact document (`@<uuid>`) or a free-text name.
#[derive(Debug, Clone, PartialEq)]
pub enum ContactRef {
    Id(Id),
    Name(String),
}

// ----------------------------------------------------------------------------
impl Serialize for ContactRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ContactRef::Id(id) => serializer.serialize_str(&format!("@{id}")),
            ContactRef::Name(name) => serializer.serialize_str(name),
        }
    }
}

// ----------------------------------------------------------------------------
impl ContactRef {
    // ------------------------------------------------------------------------
    /// `@<uuid>` as a link to a contact document, anything else as a name.
    pub fn parse(s: &str) -> ContactRef {
        match s.strip_prefix('@').and_then(|rest| Id::parse(rest).ok()) {
            Some(id) => ContactRef::Id(id),
            None => ContactRef::Name(s.to_string()),
        }
    }
}

// ----------------------------------------------------------------------------
impl<'de> Deserialize<'de> for ContactRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(ContactRef::parse(&s))
    }
}

// ----------------------------------------------------------------------------
/// Either a hex color (`#RGB` / `#RRGGBB`) or a free-text name.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorRef {
    Hex(String),
    Name(String),
}

impl ColorRef {
    // ------------------------------------------------------------------------
    /// `#…` as a hex color, anything else as a name.
    pub fn parse(s: &str) -> ColorRef {
        if s.starts_with('#') {
            ColorRef::Hex(s.to_string())
        } else {
            ColorRef::Name(s.to_string())
        }
    }
}

// ----------------------------------------------------------------------------
impl Serialize for ColorRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ColorRef::Hex(hex) => serializer.serialize_str(hex),
            ColorRef::Name(name) => serializer.serialize_str(name),
        }
    }
}

// ----------------------------------------------------------------------------
impl<'de> Deserialize<'de> for ColorRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(ColorRef::parse(&s))
    }
}

// ----------------------------------------------------------------------------
/// What the user calls a document (vault.md#document). A free-text
/// label: well-known genres get a variant, anything else is kept verbatim
/// in `Other`. Written in lowercase, parsed case-insensitively.
#[derive(Debug, Clone, PartialEq)]
pub enum Genre {
    Bill,
    Deposit,
    Expense,
    Receipt,
    Statement,
    Transfer,

    Agreement,
    Amendment,
    Contract,
    Lease,
    NDA,
    Policy,
    Terms,

    Letter,
    Note,
    Memo,
    Confirmation,
    Certificate,

    Other(String),
}

// ----------------------------------------------------------------------------
const WELL_KNOWN_GENRES: [(&str, Genre); 18] = [
    ("bill", Genre::Bill),
    ("deposit", Genre::Deposit),
    ("expense", Genre::Expense),
    ("receipt", Genre::Receipt),
    ("statement", Genre::Statement),
    ("transfer", Genre::Transfer),
    ("agreement", Genre::Agreement),
    ("amendment", Genre::Amendment),
    ("contract", Genre::Contract),
    ("lease", Genre::Lease),
    ("nda", Genre::NDA),
    ("policy", Genre::Policy),
    ("terms", Genre::Terms),
    ("letter", Genre::Letter),
    ("note", Genre::Note),
    ("memo", Genre::Memo),
    ("confirmation", Genre::Confirmation),
    ("certificate", Genre::Certificate),
];

// ----------------------------------------------------------------------------
impl Genre {
    /// Map a label to a well-known genre (case-insensitive), or `Other`.
    pub fn parse(s: &str) -> Genre {
        WELL_KNOWN_GENRES
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(s))
            .map(|(_, genre)| genre.clone())
            .unwrap_or_else(|| Genre::Other(s.to_string()))
    }

    /// The label as written to disk.
    pub fn as_str(&self) -> &str {
        match self {
            Genre::Other(s) => s,
            known => WELL_KNOWN_GENRES
                .iter()
                .find(|(_, genre)| genre == known)
                .map(|(name, _)| *name)
                .unwrap_or_default(),
        }
    }

    /// The well-known genres' labels, as written to disk.
    pub fn well_known() -> impl Iterator<Item = &'static str> {
        WELL_KNOWN_GENRES.iter().map(|(name, _)| *name)
    }
}

// ----------------------------------------------------------------------------
impl Serialize for Genre {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

// ----------------------------------------------------------------------------
impl<'de> Deserialize<'de> for Genre {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Genre::parse(&s))
    }
}

// ----------------------------------------------------------------------------
/// Which models may read an entity (vault.md#metadata): `public` any,
/// `restricted` and `pii` one on this PC only, `confidential` none. A
/// value the app doesn't know is kept, and reaches as little as
/// `confidential`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classification {
    Public,
    Pii,
    Restricted,
    Confidential,
    Other(String),
}

// ----------------------------------------------------------------------------
const CLASSIFICATIONS: [(&str, Classification); 4] = [
    ("public", Classification::Public),
    ("pii", Classification::Pii),
    ("restricted", Classification::Restricted),
    ("confidential", Classification::Confidential),
];

// ----------------------------------------------------------------------------
impl Classification {
    /// A known value (case-insensitive), or `Other`.
    pub fn parse(s: &str) -> Classification {
        CLASSIFICATIONS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(s))
            .map(|(_, class)| class.clone())
            .unwrap_or_else(|| Classification::Other(s.to_string()))
    }

    /// The value as written to disk.
    pub fn as_str(&self) -> &str {
        match self {
            Classification::Other(s) => s,
            known => CLASSIFICATIONS
                .iter()
                .find(|(_, class)| class == known)
                .map(|(name, _)| *name)
                .unwrap_or_default(),
        }
    }

    /// How little it lets reach a model: `public` 0, `pii` 1 (a cloud
    /// model gets placeholders), `restricted` 2 (a local model only),
    /// `confidential` and anything unknown 3 (none). The stricter of two
    /// wins where both apply.
    pub fn rank(&self) -> u8 {
        match self {
            Classification::Public => 0,
            Classification::Pii => 1,
            Classification::Restricted => 2,
            Classification::Confidential | Classification::Other(_) => 3,
        }
    }

    /// The stricter of `self` and `other`.
    pub fn stricter(self, other: Classification) -> Classification {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

// ----------------------------------------------------------------------------
impl Serialize for Classification {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

// ----------------------------------------------------------------------------
impl<'de> Deserialize<'de> for Classification {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Classification::parse(&s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_classification_keeps_what_it_doesnt_know_and_ranks_it_strictest() {
        assert_eq!(Classification::parse("PII"), Classification::Pii);
        let secret = Classification::parse("secret");
        assert_eq!(secret.as_str(), "secret");
        assert_eq!(secret.rank(), Classification::Confidential.rank());
        assert_eq!(
            Classification::Public.stricter(Classification::Pii),
            Classification::Pii
        );
        assert_eq!(
            Classification::Restricted.stricter(Classification::Pii),
            Classification::Restricted
        );
    }

    #[test]
    fn contact_ref_round_trips_id() {
        let id = Id::generate();
        let refval = ContactRef::Id(id);
        let yaml = serde_yaml_ng::to_string(&refval).unwrap();
        assert_eq!(yaml.trim(), format!("'@{id}'"));
        let back: ContactRef = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(back, ContactRef::Id(id));
    }

    #[test]
    fn contact_ref_round_trips_name() {
        let refval = ContactRef::Name("Jane Doe".to_string());
        let yaml = serde_yaml_ng::to_string(&refval).unwrap();
        let back: ContactRef = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(back, refval);
    }

    #[test]
    fn genre_parses_case_insensitively_and_writes_lowercase() {
        let g: Genre = serde_yaml_ng::from_str("Contract").unwrap();
        assert_eq!(g, Genre::Contract);
        assert_eq!(serde_yaml_ng::to_string(&g).unwrap().trim(), "contract");

        let g: Genre = serde_yaml_ng::from_str("NDA").unwrap();
        assert_eq!(g, Genre::NDA);
        assert_eq!(g.as_str(), "nda");
    }

    #[test]
    fn genre_keeps_unknown_labels_verbatim() {
        let g: Genre = serde_yaml_ng::from_str("Tax Return").unwrap();
        assert_eq!(g, Genre::Other("Tax Return".to_string()));
        assert_eq!(serde_yaml_ng::to_string(&g).unwrap().trim(), "Tax Return");
    }

    #[test]
    fn the_well_known_genres_parse_as_themselves() {
        let names: Vec<&str> = Genre::well_known().collect();
        assert_eq!(names.len(), 18);
        assert!(names.contains(&"bill") && names.contains(&"note"));
        for name in names {
            assert_eq!(Genre::parse(name).as_str(), name);
        }
    }

    #[test]
    fn color_ref_round_trips_hex_and_name() {
        let hex: ColorRef = serde_yaml_ng::from_str("'#FF00FF'").unwrap();
        assert_eq!(hex, ColorRef::Hex("#FF00FF".to_string()));

        let name: ColorRef = serde_yaml_ng::from_str("blue").unwrap();
        assert_eq!(name, ColorRef::Name("blue".to_string()));
    }
}

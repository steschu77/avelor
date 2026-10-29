//! Generated filenames (vault.md#filenames): a presentation layer
//! over the frontmatter, `{YYYYMMDDThhmm}.{slug(title)}.{id4}.{type-id}.{ext}`.
//! Identity never depends on it -- that is the frontmatter `id`.

use crate::document::{Document, FileFormat};
use av_utils::datetime::DateTime;

// ----------------------------------------------------------------------------
const SLUG_LEN: usize = 20;

// ----------------------------------------------------------------------------
/// The generated filename for `doc`, from its `date` (now, if it has none),
/// `title`, `id`, derived type, and file format.
pub fn generated_name(doc: &Document) -> String {
    let date = doc.date().unwrap_or_else(DateTime::now);
    let prefix = fn_date(&date);
    let id = doc.id().to_string();
    let id4 = &id[id.len() - 4..];
    let ext = match doc.format() {
        FileFormat::Markdown => "md",
        FileFormat::YAML => "yaml",
    };
    format!(
        "{prefix}.{}.{id4}.{}.{ext}",
        slug(doc.title()),
        doc.kind().type_id()
    )
}

// ----------------------------------------------------------------------------
/// The name of `doc`'s original (vault.md#document): its generated name
/// with the original's extension `ext` in place of its own, so the two sort
/// together. An extension the index reads gets `.txt` after it, so the file
/// is never taken for an entity; one that isn't a short ASCII word is `bin`.
pub fn original_name(doc: &Document, ext: &str) -> String {
    let name = generated_name(doc);
    let stem = name
        .rsplit_once('.')
        .map_or(name.as_str(), |(stem, _)| stem);
    let ext = ext.to_ascii_lowercase();
    match ext.as_str() {
        "md" | "yaml" | "yml" => format!("{stem}.{ext}.txt"),
        _ if ext.is_empty() || ext.len() > 8 || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) => {
            format!("{stem}.bin")
        }
        _ => format!("{stem}.{ext}"),
    }
}

// ----------------------------------------------------------------------------
/// Lowercased, ASCII-folded, every other run of characters (spaces, dots,
/// punctuation) collapsed to one `-`, truncated to [`SLUG_LEN`]. Never
/// empty and never contains a dot, so a name always has five segments.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in title.chars().flat_map(char::to_lowercase) {
        let folded = fold(c);
        if folded.is_empty() {
            dash = !out.is_empty();
            continue;
        }
        for f in folded.chars() {
            if out.len() == SLUG_LEN {
                break;
            }
            if dash {
                out.push('-');
                dash = false;
                if out.len() == SLUG_LEN {
                    break;
                }
            }
            out.push(f);
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out.to_string()
    }
}

// ----------------------------------------------------------------------------
/// A readable folder name for `title` (e.g. a calendar's events folder):
/// the title as is, with characters Windows doesn't allow in names
/// replaced by `-` and trailing dots and spaces trimmed; [`slug`] if
/// nothing is left.
pub fn folder_name(title: &str) -> String {
    let name: String = title
        .trim()
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();
    let name = name.trim_end_matches(['.', ' ']);
    if name.trim_matches(['-', '.', ' ']).is_empty() {
        slug(title)
    } else {
        name.to_string()
    }
}

// ----------------------------------------------------------------------------
/// One lowercase character as ASCII slug characters; empty for a separator.
fn fold(c: char) -> &'static str {
    const ALNUM: &str = "abcdefghijklmnopqrstuvwxyz0123456789";
    if c.is_ascii_alphanumeric() {
        let i = ALNUM.find(c).unwrap_or(0);
        return &ALNUM[i..i + 1];
    }
    match c {
        'ä' | 'æ' => "ae",
        'ö' | 'ø' | 'œ' => "oe",
        'ü' => "ue",
        'ß' => "ss",
        'à' | 'á' | 'â' | 'ã' | 'å' | 'ā' => "a",
        'ç' | 'ć' | 'č' => "c",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' => "e",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'ñ' | 'ń' => "n",
        'ò' | 'ó' | 'ô' | 'õ' => "o",
        'ù' | 'ú' | 'û' => "u",
        'ý' | 'ÿ' => "y",
        'š' | 'ś' => "s",
        'ž' | 'ź' | 'ż' => "z",
        'ł' => "l",
        _ => "",
    }
}

// ----------------------------------------------------------------------------
/// The `YYYYMMDDThhmm` filename prefix (vault.md#filenames):
/// to the minute, `T0000` for a date without a time.
fn fn_date(d: &DateTime) -> String {
    let (year, month, day) = d.date.to_ymd();
    let month: i32 = month.into();
    let (hour, minute, _) = d.time.map(|t| t.to_hms()).unwrap_or((0, 0, 0));
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_fold_collapse_and_truncate() {
        assert_eq!(slug("ID Card Renewal"), "id-card-renewal");
        assert_eq!(slug("Meißen.Report"), "meissen-report");
        assert_eq!(slug("  Über -- straße!  "), "ueber-strasse");
        assert_eq!(
            slug("Preparing the ID card appointment"),
            "preparing-the-id-car"
        );
        assert_eq!(
            slug("abcdefghijklmnopqrs tuv"),
            "abcdefghijklmnopqrs",
            "no trailing dash"
        );
        assert_eq!(slug("???"), "untitled");
    }

    #[test]
    fn folder_names_stay_readable() {
        assert_eq!(folder_name("Personal"), "Personal");
        assert_eq!(folder_name("alex@example.com"), "alex@example.com");
        assert_eq!(
            folder_name(" Feiertage in Deutschland "),
            "Feiertage in Deutschland"
        );
        assert_eq!(folder_name("Work: Team/Ops"), "Work- Team-Ops");
        assert_eq!(folder_name("Trailing..."), "Trailing");
        assert_eq!(folder_name("???"), "untitled");
        assert_eq!(folder_name(""), "untitled");
    }

    #[test]
    fn names_follow_the_vault_formula() {
        let text = "id: 01a0c33f-4a20-7269-bc56-cf275a720bc1\ncreated: {}\nthread: {}\ntitle: \"ID Card Renewal\"\ndate: 2026-09-21T09:15:00Z\n";
        let doc = Document::load(text.into(), FileFormat::YAML);
        assert_eq!(
            generated_name(&doc),
            "20260921T0915.id-card-renewal.0bc1.thd.yaml"
        );

        let text = "---\nid: 01a06d07-2b70-708c-9c71-7ddfaa46c0f5\ntask: {}\ntitle: Renew\ndate: 2026-08-27\n---\n";
        let doc = Document::load(text.into(), FileFormat::Markdown);
        assert_eq!(generated_name(&doc), "20260827T0000.renew.c0f5.tsk.md");
    }

    #[test]
    fn an_original_is_named_after_its_document() {
        let text = "---\nid: 01a06d07-2b70-708c-9c71-7ddfaa46c0f5\ndocument: {}\ntitle: Rechnung\ndate: 2026-10-12T09:30:00Z\n---\n";
        let doc = Document::load(text.into(), FileFormat::Markdown);
        let name = |ext| original_name(&doc, ext);
        assert_eq!(name("PDF"), "20261012T0930.rechnung.c0f5.doc.pdf");
        assert_eq!(name("md"), "20261012T0930.rechnung.c0f5.doc.md.txt");
        assert_eq!(name("yml"), "20261012T0930.rechnung.c0f5.doc.yml.txt");
        assert_eq!(name(""), "20261012T0930.rechnung.c0f5.doc.bin");
        assert_eq!(name("p f"), "20261012T0930.rechnung.c0f5.doc.bin");
        assert_eq!(name("toolongext"), "20261012T0930.rechnung.c0f5.doc.bin");
    }
}

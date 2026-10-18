//! Thin typed client for the Gmail API v1 — only what the email import
//! needs: the filter the import page edits, the ids of every match, per-message header metadata
//! for the preview list, the account's labels, and the raw RFC 822 bytes
//! for the actual import (which feed the same MIME pipeline as a picked
//! `.eml` file).
//!
//! Needs `GoogleScope::gmail` access; an account connected without it gets
//! 403s here until reconnected.

use av_utils::base64::from_base64;
use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::url::encode_url_component;
use serde::{Deserialize, Serialize};

use crate::client::Client;

// ----------------------------------------------------------------------------
const BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

/// The most ids one `messages.list` page returns.
const PAGE_MAX: usize = 500;

// ----------------------------------------------------------------------------
/// What the import page filters by: every set condition must match.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct MailFilter {
    /// Label names, as the user sees them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Every label must be on the mail (otherwise any of them).
    #[serde(skip_serializing_if = "is_false")]
    pub every: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub starred: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub attachment: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub unread: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub important: bool,
    /// Sent by the account itself.
    #[serde(skip_serializing_if = "is_false")]
    pub sent: bool,
    /// Addresses or domains, any of which may match.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub from: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<String>,
    /// Free text, in subject or body; passed on verbatim, so Gmail's own
    /// operators work here too.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub words: String,
    /// The date preset the page resolved `after`/`before` from (`last`,
    /// `day`, `week`, `month`, `custom`); the query only uses the bounds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<DateTime>,
}

fn is_false(b: &bool) -> bool {
    !b
}

impl MailFilter {
    // ------------------------------------------------------------------------
    /// The Gmail search query (`q`) for this filter; empty matches every
    /// mail. Dates are unix seconds: Gmail reads a plain `after:2026/09/21`
    /// as midnight Pacific time.
    pub fn to_query(&self) -> String {
        let mut terms = Vec::new();

        let labels: Vec<String> = self
            .labels
            .iter()
            .map(|l| label_term(l))
            .filter(|l| !l.is_empty())
            .collect();
        match labels.as_slice() {
            [] => {}
            [one] => terms.push(format!("label:{one}")),
            many if self.every => terms.extend(many.iter().map(|l| format!("label:{l}"))),
            many => terms.push(format!("label:({})", many.join(" OR "))),
        }

        let flags = [
            (self.starred, "is:starred"),
            (self.attachment, "has:attachment"),
            (self.unread, "is:unread"),
            (self.important, "is:important"),
            (self.sent, "from:me"),
        ];
        terms.extend(
            flags
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, t)| t.to_string()),
        );

        terms.extend(address_term("from", &self.from));
        terms.extend(address_term("to", &self.to));

        let words = self.words.trim();
        if !words.is_empty() {
            terms.push(words.to_string());
        }
        if let Some(after) = self.after {
            terms.push(format!("after:{}", after.as_unix()));
        }
        if let Some(before) = self.before {
            terms.push(format!("before:{}", before.as_unix()));
        }
        terms.join(" ")
    }
}

// ----------------------------------------------------------------------------
/// A label name as Gmail's `label:` operator expects it: lowercase, spaces
/// and the nesting `/` as `-`.
fn label_term(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c == ' ' || c == '/' { '-' } else { c })
        .collect()
}

// ----------------------------------------------------------------------------
/// `from:x`, or `from:(x OR y)` for several; values with spaces are quoted.
fn address_term(operator: &str, values: &[String]) -> Option<String> {
    let values: Vec<String> = values
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| {
            if v.contains(char::is_whitespace) {
                format!("\"{}\"", v.replace('"', ""))
            } else {
                v.to_string()
            }
        })
        .collect();
    match values.as_slice() {
        [] => None,
        [one] => Some(format!("{operator}:{one}")),
        many => Some(format!("{operator}:({})", many.join(" OR "))),
    }
}

// ----------------------------------------------------------------------------
/// One search hit, ready for the import page's preview list.
pub struct MailHeader {
    pub id: String,
    /// Delivery time, unix milliseconds.
    pub epoch_ms: i64,
    pub from: String,
    pub subject: String,
    /// RFC 5322 Message-ID (with angle brackets), for dedup against the
    /// vault; empty when Gmail doesn't report one.
    pub message_id: String,
    pub size: u64,
}

// ----------------------------------------------------------------------------
/// A whole message as downloaded for the import.
pub struct RawMessage {
    /// The RFC 822 bytes — what an exported `.eml` would contain.
    pub bytes: Vec<u8>,
    /// Gmail's `internalDate` (unix milliseconds): when Google accepted the
    /// mail, or when it was sent from this account. Google calls it more
    /// reliable than the `Date` header.
    pub epoch_ms: i64,
}

// ----------------------------------------------------------------------------
/// A Gmail label.
#[derive(Deserialize)]
pub struct Label {
    pub id: String,
    pub name: String,
    /// `system` (INBOX, SENT, …) or `user`.
    #[serde(rename = "type", default)]
    pub kind: String,
}

// ----------------------------------------------------------------------------
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageList {
    #[serde(default)]
    messages: Vec<MessageRef>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct MessageRef {
    id: String,
}

#[derive(Deserialize)]
struct LabelList {
    #[serde(default)]
    labels: Vec<Label>,
}

// ----------------------------------------------------------------------------
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Message {
    #[serde(default)]
    internal_date: String,
    #[serde(default)]
    size_estimate: u64,
    payload: Option<Payload>,
    /// format=raw only: the whole RFC 822 message, base64url.
    raw: Option<String>,
}

impl Message {
    // ------------------------------------------------------------------------
    /// A header's value (format=metadata/full only); empty if absent.
    fn header(&self, name: &str) -> String {
        self.payload
            .as_ref()
            .and_then(|p| p.headers.iter().find(|h| h.name.eq_ignore_ascii_case(name)))
            .map(|h| h.value.clone())
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct Payload {
    #[serde(default)]
    headers: Vec<Header>,
}

#[derive(Deserialize)]
struct Header {
    name: String,
    value: String,
}

// ----------------------------------------------------------------------------
fn message_url(id: &str) -> String {
    format!("{BASE}/messages/{}", encode_url_component(id))
}

// ----------------------------------------------------------------------------
/// The ids of the mails matching `query` (Gmail query syntax), newest
/// first, at most `cap` of them; `true` if there are more.
pub fn list_message_ids(
    client: &Client,
    cancel: &Cancel,
    query: &str,
    cap: usize,
) -> Result<(Vec<String>, bool)> {
    let url = format!("{BASE}/messages");
    let mut ids = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let max = (cap - ids.len()).min(PAGE_MAX).to_string();
        let mut params = vec![("maxResults", max.as_str()), ("q", query)];
        if let Some(token) = &page_token {
            params.push(("pageToken", token));
        }
        let page: MessageList = client.get(cancel, &url, &params)?;
        ids.extend(page.messages.into_iter().map(|m| m.id));
        match page.next_page_token {
            Some(token) if ids.len() < cap => page_token = Some(token),
            more => return Ok((ids, more.is_some())),
        }
    }
}

// ----------------------------------------------------------------------------
/// The preview headers of the mail `id`.
pub fn get_header(client: &Client, cancel: &Cancel, id: &str) -> Result<MailHeader> {
    let metadata = [
        ("format", "metadata"),
        ("metadataHeaders", "From"),
        ("metadataHeaders", "Subject"),
        ("metadataHeaders", "Message-ID"),
    ];
    let msg: Message = client.get(cancel, &message_url(id), &metadata)?;
    Ok(MailHeader {
        epoch_ms: msg.internal_date.parse().unwrap_or(0),
        from: msg.header("From"),
        subject: msg.header("Subject"),
        message_id: msg.header("Message-ID").trim().to_string(),
        size: msg.size_estimate,
        id: id.to_string(),
    })
}

// ----------------------------------------------------------------------------
/// The account's own labels (not INBOX, SENT, …), by name.
pub fn list_labels(client: &Client, cancel: &Cancel) -> Result<Vec<Label>> {
    let list: LabelList = client.get(cancel, &format!("{BASE}/labels"), &[])?;
    let mut labels: Vec<Label> = list
        .labels
        .into_iter()
        .filter(|l| l.kind == "user")
        .collect();
    labels.sort_by_cached_key(|l| l.name.to_lowercase());
    Ok(labels)
}

// ----------------------------------------------------------------------------
/// The complete RFC 822 message, byte-exact, with its delivery time.
pub fn get_raw_message(client: &Client, cancel: &Cancel, id: &str) -> Result<RawMessage> {
    let msg: Message = client.get(cancel, &message_url(id), &[("format", "raw")])?;
    let raw = msg.raw.ok_or_else(|| Error::InvalidResponse {
        msg: "messages.get returned no raw payload".to_string(),
    })?;
    Ok(RawMessage {
        bytes: from_base64(&raw),
        epoch_ms: msg.internal_date.parse().unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(filter: MailFilter) -> String {
        filter.to_query()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn empty_filter_matches_everything() {
        assert_eq!(query(MailFilter::default()), "");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn labels_any_or_all() {
        let labels = vec!["Umzug".to_string(), "Rechnungen/2026 Q3".to_string()];
        let any = MailFilter {
            labels: labels.clone(),
            ..Default::default()
        };
        assert_eq!(query(any), "label:(umzug OR rechnungen-2026-q3)");
        let all = MailFilter {
            labels,
            every: true,
            ..Default::default()
        };
        assert_eq!(query(all), "label:umzug label:rechnungen-2026-q3");
        let one = MailFilter {
            labels: vec!["Umzug".into(), " ".into()],
            ..Default::default()
        };
        assert_eq!(query(one), "label:umzug");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn flags_addresses_words_and_dates() {
        let filter = MailFilter {
            starred: true,
            attachment: true,
            unread: true,
            important: true,
            sent: true,
            from: vec!["telekom.de".into(), " umzug-meier.de ".into(), "".into()],
            to: vec!["Jan Kramer".into()],
            words: " Rechnung ".into(),
            after: Some(DateTime::from_unix_millis(1_790_467_200_000)),
            before: Some(DateTime::from_unix_millis(1_790_553_600_000)),
            since: Some("custom".into()),
            ..Default::default()
        };
        assert_eq!(
            query(filter),
            "is:starred has:attachment is:unread is:important from:me \
             from:(telekom.de OR umzug-meier.de) to:\"Jan Kramer\" Rechnung \
             after:1790467200 before:1790553600"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn filter_round_trips_as_kebab_case_without_defaults() {
        let filter = MailFilter {
            labels: vec!["Umzug".into()],
            starred: true,
            since: Some("week".into()),
            after: Some(DateTime::from_unix_millis(1_790_467_200_000)),
            ..Default::default()
        };
        let json = serde_json::to_string(&filter).unwrap();
        assert_eq!(
            json,
            r#"{"labels":["Umzug"],"starred":true,"since":"week","after":"2026-09-27T00:00:00Z"}"#
        );
        assert_eq!(serde_json::from_str::<MailFilter>(&json).unwrap(), filter);
        assert_eq!(
            serde_json::from_str::<MailFilter>("{}").unwrap(),
            MailFilter::default()
        );
    }
}

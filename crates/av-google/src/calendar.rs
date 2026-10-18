//! Thin typed client for the Google Calendar API v3.

use av_utils::cancel::Cancel;
use av_utils::error::Result;
use serde::Deserialize;

use crate::client::{Client, Page};

// ----------------------------------------------------------------------------
const BASE: &str = "https://www.googleapis.com/calendar/v3";

// ----------------------------------------------------------------------------
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalendarListEntry {
    pub id: String,
    pub summary: String,
    /// The primary calendar's id is the account's email address.
    pub primary: bool,
    pub background_color: String,
    pub time_zone: String,
}

// ----------------------------------------------------------------------------
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarList {
    #[serde(default)]
    items: Vec<CalendarListEntry>,
    next_page_token: Option<String>,
}

impl Page for CalendarList {
    type Item = CalendarListEntry;
    fn into_parts(self) -> (Vec<Self::Item>, Option<String>, Option<String>) {
        (self.items, self.next_page_token, None)
    }
}

// ----------------------------------------------------------------------------
/// All calendars of the account.
pub fn list_calendars(client: &Client, cancel: &Cancel) -> Result<Vec<CalendarListEntry>> {
    let url = format!("{BASE}/users/me/calendarList");
    let listed = client.list_all::<CalendarList>(cancel, &url, &[("maxResults", "250")])?;
    Ok(listed.items)
}

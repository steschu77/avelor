//! Who a connected account is: OpenID Connect `userinfo`. Needs only the
//! identity scopes, which consent always requests (see `auth`).

use av_utils::cancel::Cancel;
use av_utils::error::Result;
use serde::Deserialize;

use crate::client::Client;

// ----------------------------------------------------------------------------
const USERINFO: &str = "https://openidconnect.googleapis.com/v1/userinfo";

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UserInfo {
    pub email: String,
    /// Display name (`profile` scope); empty if the account has none.
    pub name: String,
}

// ----------------------------------------------------------------------------
pub fn get_user_info(client: &Client, cancel: &Cancel) -> Result<UserInfo> {
    client.get(cancel, USERINFO, &[])
}

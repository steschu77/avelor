//! Google APIs for Avelor: OAuth sign-in (`auth`), one `Client`
//! per connected account (`client`), who that account is (`identity`), and
//! a thin typed module per service — Calendar.
//!
//! Everything blocks: requests go through av-http, each under the caller's
//! `av_utils::cancel::Cancel`; the caller runs them on a worker thread. Nothing here
//! touches the vault or app state; callers map between Google's types and
//! their entities.

pub mod auth;
pub mod calendar;
pub mod client;
pub mod identity;

//! Keeps the session cookies in the Windows credential store.
//! The password is never stored; only the cookies VRChat hands back.

use crate::error::{AppError, AppResult};
use crate::vrchat::client::Session;

const ACCOUNT: &str = "session";

fn entry() -> AppResult<keyring::Entry> {
    keyring::Entry::new(crate::APP_SLUG, ACCOUNT).map_err(|e| AppError::Secrets(e.to_string()))
}

pub fn load() -> AppResult<Option<Session>> {
    match entry()?.get_password() {
        Ok(text) => Ok(serde_json::from_str(&text).ok()),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(AppError::Secrets(e.to_string())),
    }
}

pub fn save(session: &Session) -> AppResult<()> {
    let text = serde_json::to_string(session).map_err(|e| AppError::Secrets(e.to_string()))?;
    entry()?
        .set_password(&text)
        .map_err(|e| AppError::Secrets(e.to_string()))
}

pub fn clear() -> AppResult<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AppError::Secrets(e.to_string())),
    }
}

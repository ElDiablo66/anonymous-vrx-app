use serde::ser::SerializeStruct;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Could not reach VRChat: {0}")]
    Network(String),
    #[error("VRChat answered with an error ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("VRChat did not accept the sign-in")]
    Unauthorized,
    #[error("A verification code is needed to finish signing in")]
    TwoFactorRequired,
    #[error("That code was not accepted")]
    BadCode,
    #[error("VRChat asked us to slow down. Try again in a minute")]
    RateLimited,
    #[error("Not signed in")]
    NotSignedIn,
    #[error("{0}")]
    CannotDo(String),
    #[error("Local history problem: {0}")]
    Storage(String),
    #[error("Windows credential store problem: {0}")]
    Secrets(String),
    #[error("Unexpected data from VRChat: {0}")]
    Parse(String),
}

impl AppError {
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::Network(_) => "network",
            AppError::Api { .. } => "api",
            AppError::Unauthorized => "unauthorized",
            AppError::TwoFactorRequired => "twoFactorRequired",
            AppError::BadCode => "badCode",
            AppError::RateLimited => "rateLimited",
            AppError::NotSignedIn => "notSignedIn",
            AppError::CannotDo(_) => "cannotDo",
            AppError::Storage(_) => "storage",
            AppError::Secrets(_) => "secrets",
            AppError::Parse(_) => "parse",
        }
    }
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("kind", self.kind())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Storage(e.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        // The request URL can carry a session token, so it is left out of the message.
        AppError::Network(e.without_url().to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;

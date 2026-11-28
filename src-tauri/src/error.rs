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
    /// VRChat answered "too many requests". Holds how many seconds it asked to wait, when known.
    #[error("{}", wait_words(*.0))]
    RateLimited(Option<u64>),
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
            AppError::RateLimited(_) => "rateLimited",
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

/// "Try again in a minute", or in the time VRChat gave.
fn wait_words(seconds: Option<u64>) -> String {
    match seconds {
        Some(0 | 1) => "VRChat asked to wait. Try again in a second".into(),
        Some(s) if s <= 90 => format!("VRChat asked to wait. Try again in {s} seconds"),
        Some(s) => format!("VRChat asked to wait. Try again in {} minutes", s.div_ceil(60)),
        None => "VRChat asked us to slow down. Try again in a minute".into(),
    }
}

pub type AppResult<T> = Result<T, AppError>;

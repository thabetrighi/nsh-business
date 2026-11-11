//! Storage errors. Rule errors carry the codes the UI translates.

use nb_domain::RuleError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("rules broken: {0:?}")]
    Rules(Vec<RuleError>),
    #[error("not found: {0}")]
    NotFound(&'static str),
    #[error("already exists: {0}")]
    Duplicate(&'static str),
    #[error("migration {version}: {message}")]
    Migration { version: u32, message: String },
    #[error("stored data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("operation id already used for {0}")]
    OperationReused(String),
    #[error("an operation must record at least one event")]
    NoEvent,
    #[error("corrupt data: {0}")]
    Corrupt(String),
    #[error("backup: {0}")]
    Backup(String),
    /// The actor may not do this; a sensitive action may be approved by a
    /// supervisor (`needs_approval`). The attempt is audited.
    #[error("forbidden: {permission}")]
    Forbidden {
        permission: &'static str,
        needs_approval: bool,
    },
}

impl From<RuleError> for Error {
    fn from(e: RuleError) -> Self {
        Error::Rules(vec![e])
    }
}

impl From<Vec<RuleError>> for Error {
    fn from(e: Vec<RuleError>) -> Self {
        Error::Rules(e)
    }
}

impl From<nb_core::CoreError> for Error {
    fn from(e: nb_core::CoreError) -> Self {
        Error::Rules(vec![RuleError::from(e)])
    }
}

impl Error {
    /// The rule codes of this error, for the UI (`error.<code>`).
    pub fn rule_codes(&self) -> Vec<(Option<&'static str>, &'static str)> {
        match self {
            Error::Rules(v) => v.iter().map(|e| (e.field, e.code)).collect(),
            Error::Duplicate(what) => vec![(Some(what), "duplicate")],
            Error::NotFound(what) => vec![(Some(what), "not_found")],
            Error::Forbidden {
                permission,
                needs_approval,
            } => vec![(
                Some(permission),
                if *needs_approval {
                    "access.needs_approval"
                } else {
                    "access.denied"
                },
            )],
            _ => vec![(None, "internal")],
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

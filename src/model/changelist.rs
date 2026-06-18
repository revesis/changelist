use serde::{Deserialize, Serialize};

pub type ChangelistId = String;

pub const DEFAULT_CHANGELIST_ID: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Changelist {
    pub id: ChangelistId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
}

impl Changelist {
    pub fn new_default() -> Self {
        Changelist {
            id: DEFAULT_CHANGELIST_ID.to_string(),
            name: "Default".to_string(),
            description: None,
            created_at: now_rfc3339(),
        }
    }

    pub fn new(id: ChangelistId, name: String, description: Option<String>) -> Self {
        Changelist {
            id,
            name,
            description,
            created_at: now_rfc3339(),
        }
    }
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

pub fn new_changelist_id() -> ChangelistId {
    uuid::Uuid::new_v4().to_string()[..8].to_string()
}

// Artist model

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    pub id: i64,
    pub name: String,
    pub name_sort: Option<String>,
    pub bio: Option<String>,
}

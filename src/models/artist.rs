// Artist model

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artist {
    pub id: i64,
    pub name: String,
    pub name_sort: Option<String>,
    pub bio: Option<String>,
    pub country: Option<String>,
    pub genre: Option<String>,
    pub style: Option<String>,
    pub mood: Option<String>,
    pub formed_year: Option<i64>,
    pub born_year: Option<i64>,
    pub died_year: Option<i64>,
    pub disbanded: Option<String>,
    pub musicbrainz_id: Option<String>,
    pub theaudiodb_id: Option<String>,
}

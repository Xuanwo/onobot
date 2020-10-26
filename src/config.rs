use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub token: String,
    pub db: String,

    pub admin_group: i64,
    pub main_group: i64,

    pub offtopic_group: String,
    pub meta_group: String,
}

use serde::{Deserialize, Serialize};

use crate::model;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UserInfo {
    pub id: i32,
    pub username: String,
    pub email: String,
}

impl From<model::user::ModelEx> for UserInfo {
    fn from(user_model: model::user::ModelEx) -> Self {
        Self {
            id: user_model.id,
            username: user_model.username,
            email: user_model.email,
        }
    }
}

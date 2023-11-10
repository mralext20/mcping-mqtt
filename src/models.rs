use diesel::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Queryable, Selectable, Insertable, Debug, Serialize, Deserialize)]
#[diesel(table_name = crate::schema::servers)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct MinecraftServer {
    pub name: String,
    pub host: String,
}

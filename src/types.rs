use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinecraftServer {
    pub name: String,
    pub host: String,
}

pub trait JavaResponse: serde::Serialize {
    fn version(&self) -> String;
    fn players(&self) -> String;
    fn description(&self) -> String;
    fn favicon(&self) -> String;
}

use crate::types;
use std::fs::File;
use std::io::{Read, Write};

const CONFIG_FILE: &str = "config.json";

pub fn get_servers() -> Vec<types::MinecraftServer> {
    let mut servers = Vec::new();
    let mut file = match File::open(CONFIG_FILE) {
        Ok(file) => file,
        Err(_) => {
            let mut file = File::create(CONFIG_FILE).unwrap();
            file.write_all(serde_json::json!({"servers": []}).to_string().as_bytes())
                .unwrap();
            File::open(CONFIG_FILE).unwrap()
        }
    };
    let mut data = String::new();
    file.read_to_string(&mut data).unwrap();
    let json: serde_json::Value = serde_json::from_str(&data).unwrap();
    for server in json["servers"].as_array().unwrap() {
        servers.push(types::MinecraftServer {
            name: server["name"].as_str().unwrap().to_string(),
            host: server["host"].as_str().unwrap().to_string(),
        });
    }
    servers
}

pub fn add_server(server: types::MinecraftServer) {
    let mut file = File::open(CONFIG_FILE).unwrap();
    let mut data = String::new();
    file.read_to_string(&mut data).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&data).unwrap();
    json["servers"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": server.name,
            "host": server.host
        }));
    let mut file = File::create(CONFIG_FILE).unwrap();
    file.write_all(json.to_string().as_bytes()).unwrap();
}

pub fn delete_server(server: types::MinecraftServer) {
    let mut file = File::open(CONFIG_FILE).unwrap();
    let mut data = String::new();
    file.read_to_string(&mut data).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&data).unwrap();
    let mut servers = Vec::new();
    for s in json["servers"].as_array().unwrap() {
        if s["name"].as_str().unwrap() != server.name {
            servers.push(s);
        }
    }
    json["servers"] = serde_json::json!(servers);
    let mut file = File::create(CONFIG_FILE).unwrap();
    file.write_all(json.to_string().as_bytes()).unwrap();
}

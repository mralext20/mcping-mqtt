use crate::{models::MinecraftServer, schema::servers::table};
use diesel::prelude::*;

use diesel_async::{AsyncPgConnection, RunQueryDsl};
use mcping::{tokio::get_status, Java, JavaResponse};
use rumqttc::AsyncClient;
use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::mqtt;

use log::{debug, error, info};

pub async fn server_checking_loop(db_conn: &mut AsyncPgConnection, mqtt: Arc<AsyncClient>) {
    let servers: Vec<MinecraftServer> = table
        .select(MinecraftServer::as_select())
        .load(db_conn)
        .await
        .expect("Failed to load servers from database");

    let mut server_map: HashMap<String, Vec<String>> = HashMap::new();

    for server in servers.iter() {
        if server_map.contains_key(server.host.as_str()) {
            server_map
                .get_mut(&server.host.to_string())
                .unwrap()
                .push(server.name.clone());
        } else {
            server_map.insert(server.host.clone(), vec![server.name.clone()]);
        }
    }

    for (host, listeners) in server_map.into_iter() {
        let mqtt = mqtt.clone();
        tokio::task::spawn(check_server(host, listeners, mqtt));
    }
}

pub async fn check_server(
    host: String,
    listeners: Vec<String>,
    mqtt: Arc<AsyncClient>,
) -> Result<JavaResponse, Box<dyn std::error::Error + Send>> {
    debug!("Checking {}", host);
    let (latency, data) = match get_status(Java {
        server_address: host.to_string(),
        timeout: Some(Duration::from_secs(5)),
    })
    .await
    {
        Ok(data) => {
            info!(
                "{} is up, reporting for {} listeners",
                host,
                listeners.len()
            );
            for server in listeners.iter() {
                mqtt::post_to_mqtt(&mqtt, &format!("mcping/{}", server).to_string(), "up").await;
            }
            data
        }
        Err(e) => {
            error!(
                "{}: Server is offline: {}, posting for {} listeners",
                host,
                e,
                listeners.len()
            );
            for server in listeners.iter() {
                mqtt::post_to_mqtt(&mqtt, &format!("mcping/{}", server).to_string(), "down").await;
            }

            return Err(Box::new(e));
        }
    };
    let players_str = if data.players.online > 0 && data.players.sample.is_some() {
        data.players
            .sample
            .as_ref()
            .unwrap()
            .iter()
            .fold(String::new(), |mut acc, player| {
                acc.push_str(format!("{}, ", player.name).as_str());
                acc
            })
    } else {
        "No players online".to_string()
    };

    let entries = [
        ("host", host.to_string()),
        ("latency", latency.to_string()),
        ("version", data.version.name.clone()),
        ("description", data.description.text().to_string()),
        ("player_count", data.players.online.to_string()),
        ("player_max", data.players.max.to_string()),
        ("players", players_str),
    ];

    let mut output_string = String::new();
    for server in listeners.iter() {
        for (key, value) in entries.iter() {
            mqtt::post_to_mqtt(&mqtt, &format!("mcping/{}/{}", server, key), value).await;
            output_string.push_str(format!("{}: {}\n", key, value).as_str());
        }
    }
    debug!("{}", output_string);

    return Ok(data);
}

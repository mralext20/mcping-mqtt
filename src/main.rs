mod models;
pub mod schema;

use diesel::prelude::*;

use crate::schema::*;

use models::MinecraftServer;

use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use mcping::{tokio::get_status, Java, JavaResponse};
use rumqttc::{AsyncClient, Event, EventLoop, Incoming, MqttOptions, QoS};
use std::{collections::HashMap, env, sync::Arc, time::Duration};
use tokio::task::JoinHandle;

use time::OffsetDateTime;

use log::{debug, error, info};

#[tokio::main]
async fn main() {
    env_logger::builder()
        .filter_module("mcping_mqtt", log::LevelFilter::Debug)
        .init();

    dotenvy::dotenv().ok();
    let keys = [
        "MQTT_HOST",
        "MQTT_USERNAME",
        "MQTT_PASSWORD",
        "DATABASE_URL",
    ];
    for key in keys.iter() {
        if env::var(key).is_err() {
            panic!("{} is not set", key);
        }
    }

    let check_freq = Duration::from_secs(
        env::var("CHECK_FREQ")
            .unwrap_or("60".to_string())
            .parse::<u64>()
            .expect("CHECK_FREQ must be a number"),
    );

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db_conn = AsyncPgConnection::establish(&database_url)
        .await
        .expect("could not connect to database");

    let (mqtt, ev) = mqtt_setup(db_conn);

    match mqtt.subscribe("mcping/#", QoS::AtLeastOnce).await {
        Ok(_) => info!("Subscribed to mcping/#"),
        Err(e) => error!("Failed to subscribe to mcping/#: {}", e),
    };

    let mut db_conn = AsyncPgConnection::establish(&database_url)
        .await
        .expect("could not connect to database");

    loop {
        server_checking_loop(&mut db_conn, mqtt.clone()).await;
        tokio::time::sleep(check_freq).await;
        if ev.is_finished() {
            break;
        }
    }
}

async fn server_checking_loop(db_conn: &mut AsyncPgConnection, mqtt: Arc<AsyncClient>) {
    let servers: Vec<MinecraftServer> = servers::table
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

fn mqtt_setup(db_conn: AsyncPgConnection) -> (Arc<AsyncClient>, JoinHandle<()>) {
    let mut mqtt_options = MqttOptions::new(
        "mcping",
        env::var("MQTT_HOST").unwrap_or("localhost".to_string()),
        1883,
    );
    debug!("MQTT Server: {:?}", mqtt_options.broker_address());
    mqtt_options.set_keep_alive(Duration::from_secs(5));
    mqtt_options.set_credentials(
        env::var("MQTT_USERNAME").unwrap(),
        env::var("MQTT_PASSWORD").unwrap(),
    );
    mqtt_options.set_last_will(rumqttc::LastWill::new(
        "mcping/active",
        "down",
        QoS::AtLeastOnce,
        true,
    ));
    let (mqtt, eventloop) = AsyncClient::new(mqtt_options, 100);
    mqtt.try_publish("mcping/active", QoS::AtLeastOnce, true, "up")
        .expect("Failed to publish active message");
    let mqtt_rc = Arc::new(mqtt);
    let evloop = tokio::spawn(mqtt_loop(eventloop, mqtt_rc.clone(), db_conn));

    return (mqtt_rc, evloop);
}

async fn mqtt_loop(mut ev: EventLoop, mqtt: Arc<AsyncClient>, mut db_conn: AsyncPgConnection) {
    loop {
        match ev.poll().await {
            Ok(Event::Incoming(Incoming::Connect(_))) => {
                debug!("MQTT Connected");
            }
            Ok(Event::Incoming(Incoming::Publish(p))) if p.topic == "mcping/create" => {
                debug!("publish on create new server: {:?}", p.payload);
                let payload = match std::str::from_utf8(&p.payload) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to parse payload: {}", e);
                        mqtt.try_publish(
                            "mcping/create",
                            QoS::AtLeastOnce,
                            false,
                            "Failed to parse payload",
                        )
                        .expect("Failed to publish error message");
                        continue;
                    }
                };
                let server_to_add: MinecraftServer = match serde_json::from_str(payload) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to parse JSON: {}", e);
                        let example_server = MinecraftServer {
                            host: "mc.hypixel.net".to_string(),
                            name: "HyPixel".to_string(),
                        };

                        mqtt.try_publish(
                            "mcping/create",
                            QoS::AtLeastOnce,
                            false,
                            format!(
                                "{:?}: Failed to parse JSON\nuse Format {:?}",
                                OffsetDateTime::now_local(),
                                serde_json::to_string(&example_server).unwrap()
                            ),
                        )
                        .expect("Failed to publish error message");

                        continue;
                    }
                };
                if server_to_add.name == "active" {
                    error!("Server name cannot be 'active'");
                    mqtt.try_publish(
                        "mcping/create",
                        QoS::AtLeastOnce,
                        false,
                        "ERROR: Server name cannot be 'active'",
                    )
                    .expect("Failed to publish error message");
                    continue;
                }
                if server_to_add.name == "" || server_to_add.host == "" {
                    error!("Server name or host is empty");
                    mqtt.try_publish(
                        "mcping/create",
                        QoS::AtLeastOnce,
                        false,
                        "ERROR: Server name is empty",
                    )
                    .expect("Failed to publish error message");
                    continue;
                }
                let inserted = diesel::insert_into(servers::table)
                    .values(server_to_add)
                    .returning(MinecraftServer::as_select())
                    .load(&mut db_conn)
                    .await
                    .expect("Failed to insert server into database");
                match inserted.len() {
                    1 => {
                        debug!("Server added: {:?}", inserted[0]);
                        mqtt.try_publish("mcping/create", QoS::AtLeastOnce, false, "Server added")
                            .expect("Failed to publish success message");
                        tokio::task::spawn(check_server(
                            inserted[0].host.clone(),
                            vec![inserted[0].name.clone()],
                            mqtt.clone(),
                        ));
                    }
                    _ => {
                        error!("Failed to insert server into database");
                        mqtt.try_publish(
                            "mcping/create",
                            QoS::AtLeastOnce,
                            false,
                            "ERROR: Failed to insert server into database",
                        )
                        .expect("Failed to publish error message");
                        continue;
                    }
                }
            }
            Ok(Event::Incoming(Incoming::Publish(p)))
                if p.topic.starts_with("mcping/")
                    && std::str::from_utf8(&p.payload) == Ok("delete") =>
            {
                debug!("publish on delete server: {:?}", p.topic);
                let deleted_count = diesel::delete(diesel::QueryDsl::filter(
                    servers::table,
                    servers::name.eq(p.topic.replace("mcping/", "")),
                ))
                .execute(&mut db_conn)
                .await
                .expect("Failed to delete server from database");

                if deleted_count > 0 {
                    debug!("Server deleted: {:?}", p.topic.replace("mcping/", ""));
                    mqtt.try_publish(p.topic, QoS::AtLeastOnce, false, "deleted")
                        .expect("Failed to publish success message");
                } else {
                    mqtt.try_publish(p.topic.clone(), QoS::AtLeastOnce, false, "failed to delete")
                        .expect("Failed to publish error message");
                    error!("Server not found: {}", p.topic);
                    continue;
                };
            }

            Ok(_) => (),
            Err(e) => {
                error!("Connection error: {}", e);
                break;
            }
        }
    }
}

async fn post_to_mqtt(client: &AsyncClient, topic: &str, data: &str) {
    match client.try_publish(topic, QoS::AtLeastOnce, true, data) {
        Ok(_) => (),
        Err(e) => {
            error!("Failed to publish to MQTT: {}", e);
            debug!("was attempting to MQTT: {} -> {}", topic, data);
        }
    }
}

async fn check_server(
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
                post_to_mqtt(&mqtt, &format!("mcping/{}", server).to_string(), "up").await;
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
                post_to_mqtt(&mqtt, &format!("mcping/{}", server).to_string(), "down").await;
            }

            return Err(Box::new(e));
        }
    };
    let players_str: String;
    if data.players.online > 0 && data.players.sample.is_some() {
        players_str =
            data.players
                .sample
                .as_ref()
                .unwrap()
                .iter()
                .fold(String::new(), |mut acc, player| {
                    acc.push_str(format!("{}, ", player.name).as_str());
                    acc
                });
    } else {
        players_str = "No players online".to_string();
    }

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
            post_to_mqtt(&mqtt, &format!("mcping/{}/{}", server, key), value).await;
            output_string.push_str(format!("{}: {}\n", key, value).as_str());
        }
    }
    debug!("{}", output_string);

    return Ok(data);
}

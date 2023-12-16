use crate::minecraft;

use diesel::prelude::*;

use crate::schema::*;

use crate::models::MinecraftServer;

use diesel_async::{AsyncPgConnection, RunQueryDsl};
use rumqttc::{AsyncClient, Event, EventLoop, Incoming, MqttOptions, QoS};
use std::{env, sync::Arc, time::Duration};
use tokio::task::JoinHandle;

use time::OffsetDateTime;

use log::{debug, error};

pub fn setup(db_conn: AsyncPgConnection) -> (Arc<AsyncClient>, JoinHandle<()>) {
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
    let evloop = tokio::spawn(main_loop(eventloop, mqtt_rc.clone(), db_conn));

    return (mqtt_rc, evloop);
}

async fn main_loop(mut ev: EventLoop, mqtt: Arc<AsyncClient>, mut db_conn: AsyncPgConnection) {
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
                if server_to_add.name.is_empty() || server_to_add.host.is_empty() {
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
                        tokio::task::spawn(minecraft::check_server(
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

pub async fn post_to_mqtt(client: &AsyncClient, topic: &str, data: &str) {
    match client.try_publish(topic, QoS::AtLeastOnce, true, data) {
        Ok(_) => (),
        Err(e) => {
            error!("Failed to publish to MQTT: {}", e);
            debug!("was attempting to MQTT: {} -> {}", topic, data);
        }
    }
}

mod config;
mod types;

use mcping::{tokio::get_status, Java, JavaResponse};
use rumqttc::{AsyncClient, Event, EventLoop, Incoming, MqttOptions, QoS};
use std::{
    env,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinHandle;
use types::MinecraftServer;

use log::{debug, error, info};

#[tokio::main]
async fn main() {
    env_logger::builder()
        .filter_module("mcping_mqtt", log::LevelFilter::Debug)
        .init();

    let mut servers = Arc::new(Mutex::new(config::get_servers()));
    let (mqtt, ev) = mqtt_setup(servers.clone());

    match mqtt.subscribe("mcping/create", QoS::AtLeastOnce).await {
        Ok(_) => info!("Subscribed to mcping/create"),
        Err(e) => error!("Failed to subscribe to mcping/create: {}", e),
    };

    loop {
        server_checking_loop(servers.clone(), &mqtt).await;
        tokio::time::sleep(Duration::from_secs(30)).await;
        if ev.is_finished() {
            break;
        }
    }
}

async fn server_checking_loop(servers: Arc<Mutex<Vec<MinecraftServer>>>, mqtt: &AsyncClient) {
    let servers = servers.lock().unwrap();
    for server in servers.iter() {
        let server = server.clone();
        let res = check_server(server.host).await;
        match res {
            Ok(_res) => {
                info!("{} is up!", server.name);
                post_to_mqtt(&mqtt, &format!("mcping/{}", server.name).to_string(), "up").await;
            }
            Err(e) => {
                error!("{} is down: {}", server.name, e);
                post_to_mqtt(
                    &mqtt,
                    &format!("mcping/{}", server.name).to_string(),
                    "down",
                )
                .await;
                continue;
            }
        }
    }
}

fn mqtt_setup(servers: Arc<Mutex<Vec<MinecraftServer>>>) -> (Arc<AsyncClient>, JoinHandle<()>) {
    let mut mqtt_options = MqttOptions::new(
        "mcping",
        env::var("MQTT_SERVER").unwrap_or("localhost".to_string()),
        1883,
    );
    mqtt_options.set_keep_alive(Duration::from_secs(5));
    mqtt_options.set_credentials("mcping", "password");
    let (mqtt, eventloop) = AsyncClient::new(mqtt_options, 10);
    let mqtt_rc = Arc::new(mqtt);
    let evloop = tokio::spawn(mqtt_loop(eventloop, mqtt_rc.clone(), servers));

    return (mqtt_rc, evloop);
}

async fn mqtt_loop(
    mut ev: EventLoop,
    mqtt: Arc<AsyncClient>,
    servers: Arc<Mutex<Vec<MinecraftServer>>>,
) {
    loop {
        match ev.poll().await {
            Ok(Event::Incoming(Incoming::PubAck(_))) => {
                debug!("puback");
            }
            Ok(Event::Incoming(Incoming::Publish(p))) if p.topic == "mcping/create" => {
                debug!("publish on create new server: {:?}", p.payload);
                let payload = match std::str::from_utf8(&p.payload) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to parse payload: {}", e);
                        mqtt.try_publish(
                            "mcping/create/error",
                            QoS::AtLeastOnce,
                            false,
                            "Failed to parse payload",
                        )
                        .expect("Failed to publish error message");
                        continue;
                    }
                };
                let server_to_add: types::MinecraftServer = match serde_json::from_str(payload) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to parse JSON: {}", e);
                        mqtt.try_publish(
                            "mcping/create/error",
                            QoS::AtLeastOnce,
                            false,
                            format!(
                                "{:?}: Failed to parse JSON\nuse Format {:?}",
                                SystemTime::now().duration_since(UNIX_EPOCH),
                                types::MinecraftServer {
                                    host: "mc.kbrt.xyz".to_string(),
                                    name: "KBRT".to_string()
                                }
                            ),
                        )
                        .expect("Failed to publish error message");

                        continue;
                    }
                };
                config::add_server(server_to_add.clone());
                servers.clone().lock().unwrap().push(server_to_add.clone());
                debug!("Server: {:?}", server_to_add);
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
    client
        .try_publish(topic, QoS::AtLeastOnce, false, data)
        .expect("that should have worked ; failed to send message");
}

async fn check_server(server_ip: String) -> Result<JavaResponse, Box<dyn std::error::Error>> {
    debug!("Checking {}", server_ip);
    let (latency, data) = get_status(Java {
        server_address: server_ip,
        timeout: Some(Duration::from_secs(5)),
    })
    .await?;
    debug!("Latency: {:?}", latency);
    debug!("version: {:?}", data.version.name);
    debug!("description: {:?}", data.description.text());
    debug!("players: {}/{}", data.players.online, data.players.max);
    if data.players.online > 0 && data.players.sample.is_some() {
        debug!("Players:");
        let players = data.players.sample.as_ref().unwrap().iter();
        for player in players {
            debug!("  {}", player.name);
        }
    }
    return Ok(data);
}

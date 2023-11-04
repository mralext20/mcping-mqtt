mod types;

use mcping::{tokio::get_status, Java, JavaResponse};
use rumqttc::{AsyncClient, Event, EventLoop, Incoming, MqttOptions, QoS};
use std::{
    env,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::task::JoinHandle;

use log::{debug, error, info};

#[tokio::main]
async fn main() {
    env_logger::builder()
        .filter_module("mcping_mqtt", log::LevelFilter::Debug)
        .init();

    // let res = check_kbrt().await;
    // match res {
    //     Ok(_) => info!("KBRT is up!"),
    //     Err(e) => {
    //         error!("KBRT is down: {}", e);
    //         return;
    //     }
    // }
    let (mqtt, _ev) = mqtt_setup();

    match mqtt.subscribe("mcping/create", QoS::AtLeastOnce).await {
        Ok(_) => info!("Subscribed to mcping/create"),
        Err(e) => error!("Failed to subscribe to mcping/create: {}", e),
    };

    let servers = vec![
        types::MinecraftServer {
            name: "KBRT".to_string(),
            host: "10.0.6.1".to_string(),
        },
        types::MinecraftServer {
            name: "Hypixel".to_string(),
            host: "mc.hypixel.net".to_string(),
        },
    ];

    loop {
        server_checking_loop(servers.clone(), mqtt.clone()).await;
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

async fn server_checking_loop(servers: Vec<types::MinecraftServer>, mqtt: Arc<AsyncClient>) {
    for server in servers {
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

fn mqtt_setup() -> (Arc<AsyncClient>, JoinHandle<()>) {
    let mut mqtt_options = MqttOptions::new(
        "mcping",
        env::var("MQTT_SERVER").unwrap_or("localhost".to_string()),
        1883,
    );
    mqtt_options.set_keep_alive(Duration::from_secs(5));
    mqtt_options.set_credentials("mcping", "password");
    let (mqtt, eventloop) = AsyncClient::new(mqtt_options, 10);
    let mqtt_rc = Arc::new(mqtt);
    let evloop = tokio::spawn(mqtt_loop(eventloop, mqtt_rc.clone()));

    return (mqtt_rc, evloop);
}

async fn mqtt_loop(mut ev: EventLoop, mqtt: Arc<AsyncClient>) {
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
                let s: types::MinecraftServer = match serde_json::from_str(payload) {
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
                debug!("Server: {:?}", s);
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

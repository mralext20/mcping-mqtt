mod models;
pub mod schema;

use diesel_async::{AsyncConnection, AsyncPgConnection};
use rumqttc::QoS;
use std::{env, time::Duration};

mod minecraft;
mod mqtt;
use diesel::pg::{Pg, PgConnection};
use diesel::Connection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use log::{error, info};
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

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
    let mut blocking = PgConnection::establish(&database_url)
        .unwrap_or_else(|_| panic!("Error connecting to {}", database_url));
    run_migrations(&mut blocking).unwrap_or_else(|_| panic!("Error running migrations"));

    let (mqtt, ev) = mqtt::setup(db_conn);

    match mqtt.subscribe("mcping/#", QoS::AtLeastOnce).await {
        Ok(_) => info!("Subscribed to mcping/#"),
        Err(e) => error!("Failed to subscribe to mcping/#: {}", e),
    };

    let mut db_conn = AsyncPgConnection::establish(&database_url)
        .await
        .expect("could not connect to database");

    loop {
        minecraft::server_checking_loop(&mut db_conn, mqtt.clone()).await;
        tokio::time::sleep(check_freq).await;
        if ev.is_finished() {
            break;
        }
    }
}

fn run_migrations(
    connection: &mut impl MigrationHarness<Pg>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    // This will run the necessary migrations.
    //
    // See the documentation for `MigrationHarness` for
    // all available methods.
    connection.run_pending_migrations(MIGRATIONS)?;

    Ok(())
}

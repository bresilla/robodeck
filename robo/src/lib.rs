use std::time::Duration;

pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let session = zenoh::open(zenoh::Config::default()).await?;
    println!("robo: zenoh session opened");
    println!("robo: waiting for robot communication");

    let mut interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("robo: shutting down");
                break;
            }
            _ = interval.tick() => {
                let _ = session.put("zoneout/robo/heartbeat", "alive").await;
            }
        }
    }

    session.close().await?;
    Ok(())
}

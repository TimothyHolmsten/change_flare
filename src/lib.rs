//! Dynamic DNS updater for Cloudflare origin records.
//!
//! Discovers this host's public IPv4/IPv6 addresses via STUN and PATCHes
//! matching `A`/`AAAA` records through the Cloudflare DNS API.

pub mod cloudflare;
pub mod config;
pub mod error;
pub mod health;
pub mod ip;
pub mod updater;

pub use config::Config;
pub use error::Error;
pub use updater::Updater;

/// Load config, start optional health listener, and poll until SIGINT/SIGTERM.
pub fn run() -> Result<(), Error> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let config = Config::from_env()?;
    log::info!(
        "polling every {}s; zone {}; records {:?}",
        config.poll_interval.as_secs(),
        config.zone_id,
        config.record_names
    );

    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = running.clone();
    ctrlc::set_handler(move || {
        flag.store(false, std::sync::atomic::Ordering::SeqCst);
    })
    .map_err(|e| Error::Config(format!("failed to install signal handler: {e}")))?;

    let health = health::HealthState::new();
    let _listener = health::spawn_listener(config.health_bind, health.clone())?;

    let mut updater = Updater::new(config, health);
    updater.run_until(&running)
}

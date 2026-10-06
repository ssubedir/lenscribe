use std::{path::PathBuf, sync::Arc};

use lenscribe_core::{daemon::Daemon, http::ApiServer, Core};

struct ConsoleLogger;
static CONSOLE_LOGGER: ConsoleLogger = ConsoleLogger;

impl log::Log for ConsoleLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().starts_with("lenscribe") && metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "[{}][{}] {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if log::set_logger(&CONSOLE_LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "--config")
    {
        if !(2..=3).contains(&arguments.len()) {
            return Err("Usage: lenscribe-core --config <settings-file> [database-path]".into());
        }
        let settings_path = std::path::absolute(&arguments[1])?;
        let database = arguments
            .get(2)
            .map(PathBuf::from)
            .unwrap_or_else(|| settings_path.with_file_name("index.wedb"));
        let core = Arc::new(Core::open(database)?);
        let daemon = Daemon::load(
            core,
            settings_path,
            Arc::new(|event| {
                if let Ok(event) = serde_json::to_string(&event) {
                    eprintln!("{event}");
                }
            }),
        )?;
        let mut status = daemon.start().await?;
        if !status.settings.extraction.api_key.is_empty() {
            status.settings.extraction.api_key = "[redacted]".into();
        }
        println!("{}", serde_json::to_string_pretty(&status)?);
        tokio::signal::ctrl_c().await?;
        daemon.shutdown().await?;
        return Ok(());
    }
    if arguments.is_empty() || arguments.len() > 3 {
        eprintln!("Usage: lenscribe-core <image-folder> [database-path] [port]\n       lenscribe-core --config <settings-file> [database-path]\nDefault port: 47831. Use --config to enable AI extraction.");
        std::process::exit(2);
    }
    let database = arguments
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".lenscribe/index.wedb"));
    let port = arguments
        .get(2)
        .map(|port| port.parse::<u16>())
        .transpose()?
        .unwrap_or(47831);
    let core = Arc::new(Core::open(database)?);
    let report = core.watch_folder(
        &arguments[0],
        Arc::new(|event| match serde_json::to_string(&event) {
            Ok(event) => eprintln!("{event}"),
            Err(error) => eprintln!("Cannot serialize watch event: {error}"),
        }),
    )?;
    let server = ApiServer::start(core.clone(), port).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    println!("Read-only API: {}", server.url());
    tokio::signal::ctrl_c().await?;
    core.unwatch_folder(report.folder.id)?;
    server.stop().await;
    core.persist()?;
    Ok(())
}

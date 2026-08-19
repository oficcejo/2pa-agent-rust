use clap::Parser;
use okx_2pa_agent::config::paths::{ensure_dirs, settings_json_path};
use okx_2pa_agent::config::settings::Settings;
use okx_2pa_agent::web::server::run_server;
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "okx-2pa-agent", version = "0.3.1", about = "OKX AI Trading Agent in Rust")]
struct Args {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    #[arg(long, default_value_t = 8088)]
    port: u16,

    #[arg(long)]
    config: Option<PathBuf>,
}

fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

struct LocalTimer;

impl tracing_subscriber::fmt::time::FormatTime for LocalTimer {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,okx_2pa_agent=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_timer(LocalTimer))
        .init();

    let args = Args::parse();
    ensure_dirs();

    let config_path = args.config.unwrap_or_else(settings_json_path);
    let settings = Settings::load_from_file_and_env(&config_path);

    let has_env = std::path::Path::new(".env").exists();
    let is_configured = settings.is_provider_configured();

    println!("====================================================");
    println!("  OKX 2PA Agent (Rust High-Performance Edition)");
    println!("  Version: 0.3.1");
    println!("  Listen: http://{}:{}", args.host, args.port);
    println!("====================================================");

    if !has_env || !is_configured {
        println!();
        println!("  ⚠️ [系统提示] 未检测到 .env 配置文件或 API 密钥未配置。");
        println!("  🌐 正在自动为您打开 Web 配置向导: http://{}:{}", args.host, args.port);
        println!("  📝 在浏览器中配置好后将自动保存至根目录 .env，方便后续直接启动。");
        println!();
        let url = format!("http://{}:{}", args.host, args.port);
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;
            open_browser(&url);
        });
    }

    run_server(&args.host, args.port, settings).await?;
    Ok(())
}

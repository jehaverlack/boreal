macro_rules! println {
    () => {
        std::println!()
    };

    ($($argument:tt)*) => {
        log::info!($($argument)*)
    };
}

macro_rules! eprintln {
    ($($argument:tt)*) => {
        log::error!($($argument)*)
    };
}

mod app;
mod bootstrap;
mod config;
mod database;
mod desktop;
mod github;
mod google;
mod keeper;
mod local_files;
mod logging;
mod rclone;
mod s3;
mod update;
mod web;

use std::{error::Error, sync::Arc, time::Duration};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use app::AppState;

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn main() {
    desktop::run_native(|| {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                std::eprintln!("Unable to start the BOREAL async runtime: {error}");
                return;
            }
        };
        let result = runtime.block_on(run_boreal());
        runtime.shutdown_timeout(Duration::from_secs(2));
        if let Err(error) = result {
            std::eprintln!("BOREAL stopped with an error: {error}");
        }
    })
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn main() -> Result<(), Box<dyn Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run_boreal());
    runtime.shutdown_timeout(Duration::from_secs(2));
    result
}

async fn run_boreal() -> Result<(), Box<dyn Error>> {
    let runtime = bootstrap::initialize()?;

    logging::initialize(&runtime)?;

    let webapp = config::get_webapp_config(&runtime.boreal)?;
    let browser_host = match webapp.listen.as_str() {
        "::1" => "[::1]",
        other => other,
    };
    let web_url = format!("http://{}:{}", browser_host, webapp.port);
    desktop::set_web_url(web_url.clone());

    if existing_instance(&webapp.listen, webapp.port).await {
        std::println!("BOREAL is already running. Opening {web_url}");
        if let Err(error) = webbrowser::open(&web_url) {
            std::eprintln!("Unable to open the existing BOREAL WebUI: {error}");
        }
        return Ok(());
    }

    if !matches!(webapp.listen.as_str(), "127.0.0.1" | "localhost" | "::1") {
        return Err("BOREAL refuses to listen on a non-local address".into());
    }
    // Reserve the port before starting services so simultaneous launches do not
    // initialize two copies of the database, tray, or Rclone services.
    let listener = match tokio::net::TcpListener::bind((webapp.listen.as_str(), webapp.port)).await
    {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline {
                if existing_instance(&webapp.listen, webapp.port).await {
                    if let Err(error) = webbrowser::open(&web_url) {
                        std::eprintln!("Unable to open the existing BOREAL WebUI: {error}");
                    }
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            return Err(error.into());
        }
        Err(error) => return Err(error.into()),
    };

    let metadata: serde_json::Value = serde_json::from_str(include_str!("../metadata.json"))?;
    let maturity = metadata
        .pointer("/METADATA/maturity")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Unknown");

    std::println!("BOREAL v{} ({maturity})", env!("CARGO_PKG_VERSION"));
    std::println!("GitHub: https://github.com/jehaverlack/boreal");
    std::println!("Startup status: Starting BOREAL services...");

    log::info!(
        "BOREAL v{} ({maturity}) starting",
        env!("CARGO_PKG_VERSION")
    );

    log::info!("BOREAL runtime directories initialized");

    let state = Arc::new(AppState::new(runtime));
    desktop::register_state(&state);

    #[cfg(all(unix, not(target_os = "macos")))]
    let desktop_tray = desktop::start_linux_tray().await;

    AppState::initialize_rclone(Arc::clone(&state));
    AppState::start_update_monitor(Arc::clone(&state));

    let web_result = web::run(Arc::clone(&state), listener).await;

    state.request_shutdown();

    state.stop_rclone_gui();

    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(desktop_tray) = desktop_tray {
        desktop_tray.shutdown().await;
    }

    std::println!();
    std::println!("BOREAL has stopped. The application has exited.");
    log::info!("BOREAL shutdown cleanup complete; application exiting");

    web_result?;

    Ok(())
}

/// Confirm that the configured local endpoint is another running BOREAL
/// instance, rather than treating every occupied port as BOREAL.
async fn existing_instance(host: &str, port: u16) -> bool {
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return false;
    }
    let authority = if host == "::1" {
        format!("[::1]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let probe = async {
        let mut stream = tokio::net::TcpStream::connect((host, port)).await.ok()?;
        let request =
            format!("GET /app/instance HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut response = Vec::new();
        stream.take(4096).read_to_end(&mut response).await.ok()?;
        let response = std::str::from_utf8(&response).ok()?;
        Some(is_boreal_instance_response(response))
    };
    tokio::time::timeout(Duration::from_secs(2), probe)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
}

fn is_boreal_instance_response(response: &str) -> bool {
    (response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200"))
        && response
            .split_once("\r\n\r\n")
            .is_some_and(|(_, body)| body.trim() == "BOREAL")
}

#[cfg(test)]
mod desktop_instance_tests {
    use super::is_boreal_instance_response;

    #[tokio::test]
    async fn detects_an_instance_with_a_fragmented_response() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nBO")
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            stream.write_all(b"REAL").await.unwrap();
        });
        assert!(super::existing_instance("127.0.0.1", port).await);
        server.await.unwrap();
        assert!(!is_boreal_instance_response(
            "HTTP/1.1 200 OK\r\n\r\nBOREAL-OTHER"
        ));
    }

    #[test]
    fn recognizes_an_existing_boreal_status_response() {
        assert!(is_boreal_instance_response(
            "HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nBOREAL"
        ));
    }

    #[test]
    fn rejects_a_non_boreal_response() {
        assert!(!is_boreal_instance_response(
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nOTHER"
        ));
    }
}

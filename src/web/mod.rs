pub mod routes;
mod xlsx;

use std::{error::Error, sync::Arc};

use axum::Router;
use tokio::sync::watch;

use crate::{app::AppState, config};

/// Run the local BOREAL WebUI.
pub async fn run(
    state: Arc<AppState>,
    listener: tokio::net::TcpListener,
) -> Result<(), Box<dyn Error>> {
    let webapp = config::get_webapp_config(&state.runtime.boreal)?;

    /*
     * BOREAL's WebUI must remain local-only.
     */
    match webapp.listen.as_str() {
        "127.0.0.1" | "localhost" | "::1" => {}

        other => {
            return Err(format!("BOREAL refuses to listen on non-local address: {other}").into());
        }
    }

    if let Ok(database) = state.database() {
        match crate::database::migration::recover_interrupted(&database) {
            Ok(recovered) if recovered > 0 => {
                log::warn!("Marked {recovered} interrupted migration(s) as resumable");
            }
            Ok(_) => {}
            Err(error) => log::error!("Unable to recover interrupted migrations: {error}"),
        }
    }

    let app: Router = routes::router().with_state(Arc::clone(&state));

    let browser_host = match webapp.listen.as_str() {
        "::1" => "[::1]",
        other => other,
    };

    let url = format!("http://{}:{}", browser_host, webapp.port,);

    std::println!("WebUI: {url}");
    std::println!("BOREAL WebUI is available. Check App > Settings for source connection status.");
    log::info!("BOREAL WebUI listening at {url}");

    if webapp.open_browser {
        if let Err(error) = webbrowser::open(&url) {
            eprintln!("Unable to open default browser: {error}");

            eprintln!("Open this URL manually: {url}");
        }
    }

    println!("Press Ctrl-C to stop BOREAL.");

    let shutdown_rx = state.shutdown_receiver();

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(Arc::clone(&state), shutdown_rx))
        .await?;

    println!("BOREAL WebUI stopped.");

    Ok(())
}

/// Wait for either:
///
/// - Ctrl-C at the terminal
/// - a shutdown request from the WebUI
async fn shutdown_signal(state: Arc<AppState>, mut shutdown_rx: watch::Receiver<bool>) {
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            match result {
                Ok(()) => {
                    println!();
                    println!(
                        "Ctrl-C received. Stopping BOREAL..."
                    );

                    state.request_shutdown();
                }

                Err(error) => {
                    eprintln!(
                        "Unable to listen for Ctrl-C: {error}"
                    );
                }
            }
        }

        _ = wait_for_shutdown_request(
            &mut shutdown_rx,
        ) => {
            println!(
                "Shutdown requested from WebUI."
            );
        }
    }
}

/// Wait until AppState indicates that shutdown has been requested.
async fn wait_for_shutdown_request(shutdown_rx: &mut watch::Receiver<bool>) {
    loop {
        if *shutdown_rx.borrow() {
            return;
        }

        if shutdown_rx.changed().await.is_err() {
            return;
        }
    }
}

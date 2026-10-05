mod app;
mod benchmark;
mod feedback;
mod game_ui;
mod options;
mod session;

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use iphone_mirror_rs::metrics::Metrics;
use iphone_mirror_rs::video::LatestFrame;
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;
use winit::event_loop::EventLoop;

fn main() -> Result<()> {
    let Some(options) = options::Options::parse()? else {
        return Ok(());
    };
    let metrics = Arc::new(Metrics::default());
    let benchmark = options
        .benchmark_out
        .as_ref()
        .map(|path| benchmark::Benchmark::new(path, &options))
        .transpose()?;
    let result = run(options, metrics.clone());
    if let Some(benchmark) = benchmark {
        benchmark.finish(&metrics, result.is_ok())?;
    }
    result
}

fn run(options: options::Options, metrics: Arc<Metrics>) -> Result<()> {
    let game = options
        .game_profile
        .clone()
        .map(game_ui::GameControls::load)
        .transpose()?;
    let _instance = iphone_mirror_rs::instance::InstanceGuard::acquire()?;
    // Third-party debug logs can include protocol payloads or credentials.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("iphone_mirror_rs=info"))
        .add_directive("idevice=off".parse()?)
        .add_directive("jktcp=off".parse()?);
    if let Some(path) = &options.trace {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .context("cannot create new trace file")?;
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_target(false)
            .init();
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let (stop_tx, stop_rx) = watch::channel(false);
    let latest = Arc::new(LatestFrame::new());
    metrics
        .measure_stamp
        .store(options.measure_stamp, std::sync::atomic::Ordering::Relaxed);
    let input = Arc::new(session::InputBus::new(metrics.clone()));
    let event_loop = if options.headless {
        None
    } else {
        Some(EventLoop::<app::AppEvent>::with_user_event().build()?)
    };
    let proxy = event_loop.as_ref().map(EventLoop::create_proxy);
    let finished_proxy = proxy.clone();
    let shared = session::SessionShared {
        input: input.clone(),
        latest: latest.clone(),
        metrics: metrics.clone(),
        proxy,
    };
    let session = runtime.spawn(async move {
        let result = session::run(options.device, options.decoder, stop_rx, shared).await;
        if result.is_err() {
            tracing::error!("native session failed; see the preceding stage labels");
        }
        if let Some(proxy) = finished_proxy {
            let _ = proxy.send_event(app::AppEvent::Finished(result.is_err()));
        }
        result
    });
    let shutdown = stop_tx.clone();
    runtime.spawn(async move {
        if let Some(duration) = options.duration {
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = tokio::time::sleep(duration) => {} }
        } else { let _ = tokio::signal::ctrl_c().await; }
        let _ = shutdown.send(true);
    });
    let gui_failed = if let Some(event_loop) = event_loop {
        let mut app = app::App::new(latest, input, metrics.clone(), stop_tx.clone());
        app.game = game;
        app.presentation = options.presentation;
        event_loop.run_app(&mut app)?;
        app.failed
    } else {
        false
    };
    if !options.headless {
        let _ = stop_tx.send(true);
    }
    let result = runtime.block_on(async {
        if options.headless {
            session.await.context("session task failed")?
        } else {
            tokio::time::timeout(Duration::from_secs(15), session)
                .await
                .context("session shutdown deadline exceeded")?
                .context("session task failed")?
        }
    });
    match result {
        Ok(()) if !gui_failed => Ok(()),
        Ok(()) => anyhow::bail!("GPU viewer failed"),
        Err(error) => {
            tracing::error!(error = %format!("{error:#}"), "session error");
            Err(error.context("native mirror session did not complete successfully"))
        }
    }
}

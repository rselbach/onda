//! `models`: list, download, validate, delete (M3, DL-1..DL-11).

use std::io::Write;
use std::path::Path;

use anyhow::{Result, bail};
use onda_asr_coreml::manifest::manifest;
use onda_domain::download_progress::status_text;
use onda_domain::models::{Catalog, ModelId};
use onda_models::{DownloadEvent, ModelManager};
use tokio::sync::mpsc::UnboundedReceiver;

/// `models` subcommands.
#[derive(clap::Subcommand)]
pub enum Command {
    /// Catalog models with download size and status.
    List,
    /// Download a model; Ctrl-C cancels and keeps partial data.
    Download {
        /// Model id, e.g. parakeet-tdt-v2.
        #[arg(value_parser = parse_model)]
        model: ModelId,
    },
    /// Check a model folder against its manifest.
    Validate {
        /// Model id.
        #[arg(value_parser = parse_model)]
        model: ModelId,
    },
    /// Delete a model folder.
    Delete {
        /// Model id.
        #[arg(value_parser = parse_model)]
        model: ModelId,
    },
}

pub fn parse_model(id: &str) -> Result<ModelId, String> {
    ModelId::parse(id).ok_or_else(|| {
        let known: Vec<_> = ModelId::ALL.iter().map(|m| m.as_str()).collect();
        format!("unknown model '{id}' (known: {})", known.join(", "))
    })
}

pub fn run(command: Command, models_dir: &Path) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let (manager, events) = ModelManager::new(models_dir, manifest)?;
        let catalog = Catalog::new(None);
        match command {
            Command::List => list(&manager, &catalog),
            Command::Download { model } => download(&manager, events, &catalog, model).await,
            Command::Validate { model } => validate(&manager, model),
            Command::Delete { model } => {
                manager.delete(model)?;
                println!("deleted {}", manager.model_dir(model).display());
                Ok(())
            }
        }
    })
}

fn list(manager: &ModelManager, catalog: &Catalog) -> Result<()> {
    println!("models dir: {}", manager.models_dir().display());
    let downloaded = manager.downloaded_models();
    for info in catalog.models() {
        // built-in engines download nothing
        if manifest(info.id).is_none() {
            continue;
        }
        let status = match downloaded.contains(&info.id) {
            true => "downloaded",
            false => "not downloaded",
        };
        println!(
            "{:<18} {:<18} {:>8}  {status}",
            info.id.as_str(),
            info.name,
            info.size_hint,
        );
    }
    Ok(())
}

async fn download(
    manager: &ModelManager,
    mut events: UnboundedReceiver<DownloadEvent>,
    catalog: &Catalog,
    model: ModelId,
) -> Result<()> {
    let name = catalog.name(model);
    let print = |event: DownloadEvent| {
        print!("\r\x1b[2K{}", status_text(name, event.status()));
        if event.end().is_some() {
            println!();
        }
        std::io::stdout().flush().ok();
    };
    let ctrl_c_manager = manager.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            ctrl_c_manager.cancel(model);
        }
    });
    let download = manager.download(model);
    tokio::pin!(download);
    let result = loop {
        tokio::select! {
            result = &mut download => break result,
            Some(event) = events.recv() => print(event),
        }
    };
    // every event is sent before the download returns
    while let Ok(event) = events.try_recv() {
        print(event);
    }
    println!("{}", result?.display());
    Ok(())
}

fn validate(manager: &ModelManager, model: ModelId) -> Result<()> {
    let Some(validation) = manager.validate(model) else {
        bail!("{model} downloads nothing");
    };
    match validation.failure_details() {
        None => {
            println!(
                "{model}: complete in {}",
                manager.model_dir(model).display()
            );
            Ok(())
        }
        Some(details) => bail!("{model}: incomplete: {details}"),
    }
}

use clap::Args;
use models::entity::library;
use sea_orm::{prelude::*, Condition};
use stump_core::{
	config::StumpConfig,
	job::{run_job_inline, CoreJobOutput, StumpJob},
	CoreEvent, Ctx,
};

use super::default_progress_spinner;
use crate::{error::CliResult, CliError};

/// Scan one or more libraries, including any follow-up jobs such as thumbnail generation.
/// This runs entirely in this process, so the server does not need to be running
#[derive(Args, Debug)]
pub struct Scan {
	/// The name or ID of a library to scan. Can be provided multiple times. If omitted,
	/// all libraries are scanned
	#[clap(long)]
	library: Vec<String>,
}

pub async fn handle_scan_command(scan: Scan, config: &StumpConfig) -> CliResult<()> {
	let ctx = Ctx::new(config.clone()).await;

	let mut query = library::Entity::find();
	if !scan.library.is_empty() {
		query = query.filter(
			Condition::any()
				.add(library::Column::Id.is_in(scan.library.clone()))
				.add(library::Column::Name.is_in(scan.library.clone())),
		);
	}
	let libraries = query.all(ctx.conn.as_ref()).await?;

	let missing = scan
		.library
		.iter()
		.filter(|wanted| {
			!libraries
				.iter()
				.any(|l| &l.id == *wanted || &l.name == *wanted)
		})
		.collect::<Vec<_>>();
	if !missing.is_empty() {
		return Err(CliError::OperationFailed(format!(
			"Could not find libraries: {missing:?}"
		)));
	}
	if libraries.is_empty() {
		println!("No libraries to scan");
		return Ok(());
	}

	let mut failures = vec![];
	for library in libraries {
		let progress = default_progress_spinner();
		progress.set_message(format!("Scanning {}", library.name));

		// Mirror the job events (the same ones the web app shows) onto the spinner
		let mut printed_summary = false;
		let mut report = |event: CoreEvent| match event {
			CoreEvent::JobUpdate(update) => {
				if let Some(message) = update.payload.message {
					let subtitle = update.payload.subtitle.unwrap_or_default();
					progress
						.set_message(format!("{}: {message} {subtitle}", library.name));
				}
			},
			CoreEvent::JobOutput(event) if !printed_summary => {
				if let CoreJobOutput::LibraryScan(output) = event.output {
					printed_summary = true;
					// Note: Printed directly so it is still visible without a terminal
					// (e.g. from cron), where the spinner is hidden
					progress.suspend(|| {
						println!(
							"{}: {} series created, {} series updated, {} books created, {} books updated",
							library.name,
							output.created_series,
							output.updated_series,
							output.created_media,
							output.updated_media
						)
					});
				}
			},
			_ => {},
		};

		let mut events = ctx.get_client_receiver();
		let job = run_job_inline(
			ctx.apalis_state.clone(),
			StumpJob::library_scan(library.id.clone(), library.path.clone(), None),
		);
		tokio::pin!(job);
		let job_failures = loop {
			tokio::select! {
				biased;
				event = events.recv() => {
					if let Ok(event) = event {
						report(event);
					}
				},
				job_failures = &mut job => break job_failures,
			}
		};
		while let Ok(event) = events.try_recv() {
			report(event);
		}

		progress.finish_and_clear();

		if job_failures.is_empty() {
			println!("{}: done", library.name);
		} else {
			println!("{}: failed", library.name);
			for (job, error) in job_failures {
				failures.push(format!("{} ({job}): {error}", library.name));
			}
		}
	}

	if failures.is_empty() {
		Ok(())
	} else {
		Err(CliError::OperationFailed(failures.join("\n")))
	}
}

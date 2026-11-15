use crate::data::CoreContext;
use async_graphql::{Context, Result, Subscription};
use stump_core::CoreEvent;
use tokio::sync::broadcast::error::RecvError;

#[derive(Default)]
pub struct EventSubscription;

#[Subscription]
impl EventSubscription {
	async fn read_events(
		&self,
		ctx: &Context<'_>,
	) -> impl futures_util::Stream<Item = Result<CoreEvent>> {
		let mut client_recv = None;
		if let Ok(ctx) = ctx.data::<CoreContext>() {
			client_recv = Some(ctx.get_client_receiver());
		}

		async_stream::stream! {
			let Some(mut rx) = client_recv else {
				tracing::error!("Failed to get client receiver from context!");
				return;
			};

			loop {
				match rx.recv().await {
					Ok(event) => yield Ok(event),
					// Note: A slow client (e.g. a backgrounded browser tab) can fall behind during
					// a busy job. Ending the stream here would leave that client without any
					// events until it reloads, so we skip what was missed and carry on
					Err(RecvError::Lagged(skipped)) => {
						tracing::warn!(skipped, "Event subscriber lagged, skipping missed events");
					},
					Err(RecvError::Closed) => break,
				}
			}
		}
	}
}

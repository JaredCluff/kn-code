use axum::response::sse::{Event, Sse};
use futures::StreamExt;
use std::convert::Infallible;
use tokio_stream::wrappers::ReceiverStream;

pub fn jsonl_stream(
    rx: tokio::sync::mpsc::Receiver<String>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let stream = ReceiverStream::new(rx).map(|line| Ok(Event::default().data(line)));
    Sse::new(stream)
}

use futures::{Async, Poll, Stream};
use std::collections::VecDeque;

pub trait StreamExt: Stream {
    fn prepend(self, first: VecDeque<Self::Item>) -> Prepend<Self>
    where
        Self: Sized,
    {
        new(self, first)
    }
}

impl<T: ?Sized> StreamExt for T where T: Stream {}

/// An adapter for chaining the output of two streams.
///
/// The resulting stream produces items from first stream and then
/// from second stream.
#[must_use = "streams do nothing unless polled"]
pub struct Prepend<S>
where
    S: Stream,
{
    stream: S,
    session: VecDeque<<S as Stream>::Item>,
}

pub fn new<S>(stream: S, session: VecDeque<<S as Stream>::Item>) -> Prepend<S>
where
    S: Stream,
{
    Prepend { stream, session }
}

impl<S> Prepend<S>
where
    S: futures::Stream,
{
    pub fn merge_session(&mut self, session: VecDeque<<S as Stream>::Item>) {
        self.session.extend(session)
    }
}

impl<S> Stream for Prepend<S>
where
    S: Stream,
{
    type Item = <S as Stream>::Item;
    type Error = <S as Stream>::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some(v) = self.session.pop_front() {
            return Ok(Async::Ready(Some(v)));
        }

        self.stream.poll()
    }
}

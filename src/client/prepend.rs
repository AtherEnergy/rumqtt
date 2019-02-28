use futures::{Async, Poll, Stream};
use std::collections::VecDeque;
use std::iter::IntoIterator;

pub trait Prepend: Stream {
    fn prependable(self) -> Prependable<Self>
    where
        Self: Sized,
    {
        new(self)
    }
}

impl<T: ?Sized> Prepend for T where T: Stream {}

#[must_use = "streams do nothing unless polled"]
pub struct Prependable<S>
where
    S: Stream,
{
    stream: S,
    items: VecDeque<<S as Stream>::Item>,
}

pub fn new<S>(stream: S) -> Prependable<S>
where
    S: Stream,
{
    Prependable {
        stream,
        items: VecDeque::new(),
    }
}

impl<S> Prependable<S>
where
    S: futures::Stream,
{
    /// Insert items in between present items and wrapped stream
    pub fn insert(&mut self, items: impl IntoIterator<Item = <S as Stream>::Item>) {
        self.items.extend(items)
    }
}

impl<S> Stream for Prependable<S>
where
    S: Stream,
{
    type Item = <S as Stream>::Item;
    type Error = <S as Stream>::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if let Some(v) = self.items.pop_front() {
            Ok(Async::Ready(Some(v)))
        } else {
            self.stream.poll()
        }
    }
}

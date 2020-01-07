use futures::Stream;
use std::{
    collections::VecDeque,
    iter::IntoIterator,
    marker::Unpin,
    pin::Pin,
    task::{Context, Poll},
};

pub trait Prepend: Stream + Unpin {
    fn prependable(self) -> Prependable<Self>
    where
        Self: Sized,
    {
        new(self)
    }
}

impl<T: ?Sized> Prepend for T where T: Stream + Unpin {}

#[must_use = "streams do nothing unless polled"]
pub struct Prependable<S>
where
    S: Stream + Unpin,
{
    stream: S,
    items: VecDeque<<S as Stream>::Item>,
}

impl<S> Unpin for Prependable<S> where S: Stream + Unpin {}

pub fn new<S>(stream: S) -> Prependable<S>
where
    S: Stream + Unpin,
{
    Prependable {
        stream,
        items: VecDeque::new(),
    }
}

impl<S> Prependable<S>
where
    S: Stream + Unpin,
{
    /// Insert items in between present items and wrapped stream
    pub fn insert(&mut self, items: impl IntoIterator<Item = <S as Stream>::Item>) {
        self.items.extend(items)
    }
}

impl<S> Stream for Prependable<S>
where
    S: Stream + Unpin,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let Prependable { items, stream } = Pin::into_inner(self);
        if let Some(v) = items.pop_front() {
            Poll::Ready(Some(v))
        } else {
            Pin::new(stream).poll_next(cx)
        }
    }
}

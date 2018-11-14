use futures::Stream;

pub trait StreamExt: Stream {
    fn chain2<S>(self, stream: S) -> prepend::Prepend<Self, S>
        where S: Stream<Item = Self::Item, Error = Self::Error>,
              Self: Sized
    {
        prepend::new(self, stream)
    }
}

impl<T: ?Sized> StreamExt for T where T: Stream {}

pub mod prepend {
    use core::mem;
    use futures::Async;
    use futures::Poll;
    use futures::Stream;

    /// State of chain stream.
    #[derive(Debug)]
    enum State<S1, S2> {
        /// Emitting elements of first stream
        First(S1, S2),
        /// Emitting elements of second stream
        Second(S2),
        /// Temporary value to replace first with second
        Temp,
    }

    /// An adapter for chaining the output of two streams.
    ///
    /// The resulting stream produces items from first stream and then
    /// from second stream.
    #[derive(Debug)]
    #[must_use = "streams do nothing unless polled"]
    pub struct Prepend<S1, S2> {
        state: State<S1, S2>,
    }

    pub fn new<S1, S2>(s1: S1, s2: S2) -> Prepend<S1, S2>
        where S1: Stream,
              S2: Stream<Item = S1::Item, Error = S1::Error>
    {
        Prepend { state: State::First(s1, s2) }
    }

    impl<S1, S2> Prepend<S1, S2> {
//        pub fn update_first(&mut self, s: S) -> Self {
//            match self.state() {
//                State::First(s1, s2) => self::new(s, s2),
//                State::Second(s2) => self::new(s, s2),
//                State::Temp => unreachable!(),
//            }
//        }
    }

    impl<S1, S2> Stream for Prepend<S1, S2>
        where S1: Stream,
              S2: Stream<Item = S1::Item, Error = S1::Error>
    {
        type Item = S1::Item;
        type Error = S1::Error;

        fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
            loop {
                match self.state {
                    State::First(ref mut s1, ref _s2) => match s1.poll() {
                        Ok(Async::Ready(None)) => (), // roll
                        x => return x,
                    },
                    State::Second(ref mut s2) => return s2.poll(),
                    State::Temp => unreachable!(),
                }

                self.state = match mem::replace(&mut self.state, State::Temp) {
                    State::First(_s1, s2) => State::Second(s2),
                    _ => unreachable!(),
                };
            }
        }
    }
}

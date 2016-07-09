/// Queues. Note: 'record' is qos2 term for 'publish'

    /// For QoS 1. Stores outgoing publishes. Removed after `puback` is
    /// received.
    /// If 'puback' isn't received in `queue_timeout` time, client should
    /// resend these messages
    outgoing_pub: VecDeque<(i64, Box<Message>)>,

    /// For QoS 2. Store for incoming publishes to record. Released after
    /// `pubrel` is received.
    /// Client records these messages and sends `pubrec` to broker.
    /// Broker resends these messages if `pubrec` isn't received (which means
    /// either broker message
    /// is lost or clinet `pubrec` is lost) and client responds with `pubrec`.
    /// Broker also resends `pubrel` till it receives `pubcomp` (which means
    /// either borker `pubrel`
    /// is lost or client's `pubcomp` is lost)
    incoming_rec: VecDeque<Box<Message>>, 
    //
    /// For QoS 2. Store for outgoing publishes. Removed after pubrec is
    /// received.
    /// If 'pubrec' isn't received in 'timout' time, client should publish
    /// these messages again.
    outgoing_rec: VecDeque<(i64, Box<Message>)>,

    /// For Qos2. Store for outgoing `pubrel` packets. Removed after `pubcomp`
    /// is received.
    /// If `pubcomp` is not received in `timeout` time (client's `pubrel` might
    /// have been lost and message
    /// isn't released by the broker or broker's `pubcomp` is lost), client
    /// should resend `pubrel` again.
    outgoing_rel: VecDeque<(i64, PacketIdentifier)>,
use std::net::{SocketAddr, ToSocketAddrs};
use std::str;
use std::sync::Arc;
use std::thread;
use std::sync::mpsc;

use mio::*;
use mio::channel::Receiver;
use mqtt::{QualityOfService, TopicFilter};
use mqtt::control::variable_header::PacketIdentifier;

use error::Result;
use message::Message;
use clientoptions::MqttOptions;
use request::MqRequest;
use connection::{Connection, NetworkRequest, NetworkNotification, MqttState};

const RETRANSMIT_TIMER: Token = Token(1);
const PUB0_CHANNEL: Token = Token(2);
const PUB1_CHANNEL: Token = Token(3);
const PUB2_CHANNEL: Token = Token(4);
const SUB_CHANNEL: Token = Token(5);
const MISC_CHANNEL: Token = Token(7);
const NETWORK_NOTIFICATION_CHANNEL: Token = Token(6);

/// Connection state messages
pub enum MiscNwRequest {
    Disconnect,
    Shutdown,
}

// static mut N: i32 = 0;
// unsafe {
//     N += 1;
//     println!("N: {}", N);
// }

pub type MessageSendableFn = Box<Fn(Message) + Send + Sync>;
pub type PublishSendableFn = Box<Fn(Message) + Send + Sync>;

/// Handles commands from Publisher and Subscriber. Saves MQTT
/// state and takes care of retransmissions.
pub struct MqttClient {
    /// Specifies the options for MQTT client
    pub opts: MqttOptions,
    /// The connection state for the mqtt client
    pub state: MqttState,
    /// Used as an indentifier mainly for messages with QoS > 0
    pub last_pkid: PacketIdentifier,
    /// no. of pending messages in qos0 pub channel
    pub pub0_channel_pending: u32,
    /// no. of pending messages in qos1 pub channel
    pub pub1_channel_pending: u32,
    /// no. of pending messages in qos2 pub channel
    pub pub2_channel_pending: u32,
    /// QoS 0 receiver endpoint
    pub pub0_rx: Option<Receiver<Message>>,
    /// QoS 1 receiver endpoint
    pub pub1_rx: Option<Receiver<Message>>,
    /// QoS 2 receiver endpoint
    pub pub2_rx: Option<Receiver<Message>>,
    /// A list of receivers from subscriber
    pub sub_rx: Option<Receiver<Vec<(TopicFilter, QualityOfService)>>>,
    /// Sender for sending messages by the client
    pub nw_request_tx: Option<mpsc::Sender<NetworkRequest>>,
    /// Receiver for state notifications sent to the client by the broker
    pub nw_notification_rx: Option<Receiver<NetworkNotification>>,
    /// Receiver for any misc network requests.
    pub misc_rx: Option<Receiver<MiscNwRequest>>,
    /// On message callback
    pub message_callback: Option<Arc<MessageSendableFn>>,
    /// On publish callback
    pub publish_callback: Option<Arc<PublishSendableFn>>,
    /// Poll
    pub poll: Poll,
}

impl MqttClient {
    fn lookup_ipv4<A: ToSocketAddrs>(addr: A) -> SocketAddr {
        let addrs = addr.to_socket_addrs().expect("Conversion Failed");
        for addr in addrs {
            if let SocketAddr::V4(_) = addr {
                return addr;
            }
        }
        unreachable!("Cannot lookup address");
    }

    /// Create a new mqtt client with MqttOptions
    pub fn new(opts: MqttOptions) -> Self {
        // TODO: Move state initialization to MqttClient constructor
        MqttClient {
            // State
            state: MqttState::Disconnected,
            last_pkid: PacketIdentifier(0),

            opts: opts,
            pub0_channel_pending: 0,
            pub1_channel_pending: 0,
            pub2_channel_pending: 0,

            // Channels
            pub0_rx: None,
            pub1_rx: None,
            pub2_rx: None,
            sub_rx: None,
            nw_request_tx: None,
            nw_notification_rx: None,
            misc_rx: None,

            // Callbacks
            message_callback: None,
            publish_callback: None,

            // Poll
            poll: Poll::new().expect("Unable to create a poller"),
        }
    }

    /// Mock method for unit tests
    pub fn mock_start(mut self) -> Result<(Self, MqRequest, mpsc::Receiver<NetworkRequest>)> {
        let (pub0_tx, pub0_rx) = channel::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub0_rx = Some(pub0_rx);
        let (pub1_tx, pub1_rx) = channel::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub1_rx = Some(pub1_rx);
        let (pub2_tx, pub2_rx) = channel::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub2_rx = Some(pub2_rx);

        let (sub_tx, sub_rx) = channel::sync_channel::<Vec<(TopicFilter, QualityOfService)>>(self.opts.sub_q_len as usize);
        self.sub_rx = Some(sub_rx);

        let (nw_notification_tx, nw_notification_rx) = channel::sync_channel::<NetworkNotification>(5);
        self.nw_notification_rx = Some(nw_notification_rx);

        let (nw_request_tx, nw_request_rx) = mpsc::channel::<NetworkRequest>();
        self.nw_request_tx = Some(nw_request_tx);

        let (misc_tx, misc_rx) = channel::sync_channel::<MiscNwRequest>(1);
        self.misc_rx = Some(misc_rx);

        // @ Create Request through which user interacts
        // @ These are the handles using which user interacts with rumqtt.
        let mq_request = MqRequest {
            pub0_tx: pub0_tx,
            pub1_tx: pub1_tx,
            pub2_tx: pub2_tx,
            subscribe_tx: sub_tx,
            misc_tx: misc_tx,
        };

        Ok((self, mq_request, nw_request_rx))
    }

    /// Connects to the broker and starts an event loop in a new thread.
    /// Returns 'Request' and handles reqests from it.
    /// Also handles network events, reconnections and retransmissions.
    pub fn start(mut self) -> Result<MqRequest> {
        let (pub0_tx, pub0_rx) = channel::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub0_rx = Some(pub0_rx);
        let (pub1_tx, pub1_rx) = channel::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub1_rx = Some(pub1_rx);
        let (pub2_tx, pub2_rx) = channel::sync_channel::<Message>(self.opts.pub_q_len as usize);
        self.pub2_rx = Some(pub2_rx);

        let (sub_tx, sub_rx) = channel::sync_channel::<Vec<(TopicFilter, QualityOfService)>>(self.opts.sub_q_len as usize);
        self.sub_rx = Some(sub_rx);

        let (nw_notification_tx, nw_notification_rx) = channel::sync_channel::<NetworkNotification>(5);
        self.nw_notification_rx = Some(nw_notification_rx);

        let (nw_request_tx, nw_request_rx) = mpsc::channel::<NetworkRequest>();
        self.nw_request_tx = Some(nw_request_tx);

        let (misc_tx, misc_rx) = channel::sync_channel::<MiscNwRequest>(1);
        self.misc_rx = Some(misc_rx);

        // @ Create Request through which user interacts
        // @ These are the handles using which user interacts with rumqtt.
        let mq_request = MqRequest {
            pub0_tx: pub0_tx,
            pub1_tx: pub1_tx,
            pub2_tx: pub2_tx,
            subscribe_tx: sub_tx,
            misc_tx: misc_tx,
        };

        let opts = self.opts.clone();

        // This thread handles network reads (coz they are blocking) and
        // and sends them to event loop thread to handle mqtt state.
        let addr = opts.addr.clone();
        let addr = Self::lookup_ipv4(addr.as_str());
        let mut connection = try!(Connection::start(addr,
                                                    opts,
                                                    nw_request_rx,
                                                    nw_notification_tx,
                                                    self.publish_callback.clone(),
                                                    self.message_callback.clone()));
        thread::spawn(move || -> Result<()> {
            let _ = connection.run();
            error!("Network Thread Stopped !!!!!!!!!");
            Ok(())
        });

        // This is the thread that handles mio event loop.
        // Mio event loop is intentionally made to use just notify and timers.
        // Tcp Streams are std blocking streams.This helps avoiding the state
        // machine hell.
        // All the network writes also happen in this thread
        thread::spawn(move || -> Result<()> {
            // let poll = Poll::new().expect("Unable to create Poll");
            match self.run() {
                Ok(()) => println!("Event Loop returned with OK ???????"),
                Err(e) => {
                    error!("Stopping event loop. Error = {:?}", e);
                    return Err(e.into());
                }
            }
            Ok(())
        });

        Ok(mq_request)
    }

    /// Register events and add to the event set from all channels in the client
    fn poll(&mut self) -> Result<Events> {
        let events = Events::with_capacity(1024);
        let pub0_rx = self.pub0_rx.as_ref().unwrap();
        self.poll.register(pub0_rx, PUB0_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
        let pub1_rx = self.pub1_rx.as_ref().unwrap();
        self.poll.register(pub1_rx, PUB1_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
        let pub2_rx = self.pub2_rx.as_ref().unwrap();
        self.poll.register(pub2_rx, PUB2_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
        let sub_rx = self.sub_rx.as_ref().unwrap();
        self.poll.register(sub_rx, SUB_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
        let nw_notification_rx = self.nw_notification_rx.as_ref().unwrap();
        self.poll
            .register(nw_notification_rx, NETWORK_NOTIFICATION_CHANNEL, Ready::readable(), PollOpt::edge())
            .expect("Poll Register Error");
        let misc_rx = self.misc_rx.as_ref().unwrap();
        self.poll.register(misc_rx, MISC_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
        Ok(events)
    }

    /// Run the client forever acting accordingly on the message received on its channels
    fn run(mut self) -> Result<()> {
        let mut events = try!(self.poll());

        loop {
            try!(self.poll.poll(&mut events, None));
            for e in events.iter() {
                match e.token() {
                    PUB0_CHANNEL => {
                        try!(self.publish0(false));
                    }
                    PUB1_CHANNEL => {
                        try!(self.publish1(false));
                    }
                    PUB2_CHANNEL => {
                        try!(self.publish2(false));
                    }
                    SUB_CHANNEL => {
                        try!(self.subscribe());
                    }
                    NETWORK_NOTIFICATION_CHANNEL => {
                        try!(self.network_notification());
                    }
                    MISC_CHANNEL => {
                        try!(self.misc());
                    }
                    _ => panic!("Invalid Token"),
                };
            }
        }
    }


    // Note: Setting callback before subscriber & publisher
    // are created ensures that message callbacks are registered
    // before subscription & you don't need to pass callbacks through
    // channels (simplifies code)

    /// Set the message callback.  This is called when a message is
    /// received from the broker.
    pub fn message_callback<F>(mut self, callback: F) -> Self
        where F: Fn(Message) + Send + Sync + 'static
    {
        self.message_callback = Some(Arc::new(Box::new(callback)));
        self
    }

    /// Set the publish callback.  This is called when a published
    /// message has been sent to the broker successfully.
    pub fn publish_callback<F>(mut self, callback: F) -> Self
        where F: Fn(Message) + Send + Sync + 'static
    {
        self.publish_callback = Some(Arc::new(Box::new(callback)));
        self
    }

    /// Subscribe the topics
    fn subscribe(&mut self) -> Result<()> {
        let topics = {
            let sub_rx = self.sub_rx.as_ref().unwrap();
            try!(sub_rx.try_recv())
        };
        let nw_request_tx = self.nw_request_tx.as_ref().unwrap();
        try!(nw_request_tx.send(NetworkRequest::Subscribe(topics)));
        Ok(())
    }

    /// Publish a message with QoS 0
    fn publish0(&mut self, clear_backlog: bool) -> Result<()> {
        // Increment only if notificication is from publisher
        if !clear_backlog {
            self.pub0_channel_pending += 1;
        }

        debug!(" * Pub QoS0. Clear Backlog = {}, Mqtt State = {:?}, Channel pending = {}",
               clear_backlog,
               self.state,
               self.pub0_channel_pending);
        // Receive from publish qos0 channel only when connected.
        if self.state == MqttState::Connected {
            loop {
                if self.pub0_channel_pending == 0 {
                    break;
                }
                let message = {
                    let pub0_rx = self.pub0_rx.as_ref().unwrap();
                    pub0_rx.try_recv().expect("Pub0 Rx Recv Error")
                };

                // NOTE: Reregister only after 'try_recv' is successful. Or else sync_channel
                // Buffer limit isn't working.
                {
                    let pub0_rx = self.pub0_rx.as_ref().unwrap();
                    self.poll.reregister(pub0_rx, PUB0_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
                }
                let nw_request_tx = self.nw_request_tx.as_ref().unwrap();
                try!(nw_request_tx.send(NetworkRequest::Publish(message)));

                self.pub0_channel_pending -= 1;
            }
        }
        Ok(())
    }

    fn publish1(&mut self, clear_backlog: bool) -> Result<()> {
        // Increment only if notificication is from publisher
        if !clear_backlog {
            self.pub1_channel_pending += 1;
        }

        debug!(" * Pub QoS1. Clear Backlog = {}, Mqtt State = {:?}, Channel pending = {}",
               clear_backlog,
               self.state,
               self.pub1_channel_pending);

        // Receive from publish qos1 channel only when outgoing pub queue
        // length is < max and in connected state
        if self.state == MqttState::Connected {
            loop {
                if self.pub1_channel_pending == 0 {
                    break;
                }
                let mut message = {
                    let pub1_rx = self.pub1_rx.as_ref().unwrap();
                    try!(pub1_rx.try_recv())
                };

                // NOTE: Reregister only after 'try_recv' is successful. Or else sync_channel
                // Buffer limit isn't working.
                {
                    let pub1_rx = self.pub1_rx.as_ref().unwrap();
                    self.poll.reregister(pub1_rx, PUB1_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
                }
                // Add next packet id to message and publish
                let PacketIdentifier(pkid) = self._next_pkid();
                message.set_pkid(pkid);

                let nw_request_tx = self.nw_request_tx.as_ref().unwrap();
                try!(nw_request_tx.send(NetworkRequest::Publish(message)));

                self.pub1_channel_pending -= 1;
            }
        }
        Ok(())
    }

    fn publish2(&mut self, clear_backlog: bool) -> Result<()> {
        // Increment only if notificication is from publisher
        if !clear_backlog {
            self.pub2_channel_pending += 1;
        }

        debug!(" * Pub QoS2. Clear Backlog = {}, Mqtt State = {:?}, Channel pending = {}",
               clear_backlog,
               self.state,
               self.pub2_channel_pending);
        // Receive from publish qos2 channel only when outgoing pub queue
        // length is < max and in connected state
        if self.state == MqttState::Connected {
            loop {
                // Before
                if self.pub2_channel_pending == 0 {
                    break;
                }
                let mut message = {
                    let pub2_rx = self.pub2_rx.as_ref().unwrap();
                    try!(pub2_rx.try_recv())
                };

                // NOTE: Reregister only after 'try_recv' is successful. Or else sync_channel
                // Buffer limit isn't working.
                {
                    let pub2_rx = self.pub2_rx.as_ref().unwrap();
                    self.poll.reregister(pub2_rx, PUB2_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
                }

                // Add next packet id to message and publish
                let PacketIdentifier(pkid) = self._next_pkid();
                message.set_pkid(pkid);

                let nw_request_tx = self.nw_request_tx.as_ref().unwrap();
                try!(nw_request_tx.send(NetworkRequest::Publish(message)));
                self.pub2_channel_pending -= 1;
            }
        }
        Ok(())
    }

    fn network_notification(&mut self) -> Result<()> {
        let notification = {
            let nw_notification_rx = self.nw_notification_rx.as_ref().unwrap();
            try!(nw_notification_rx.try_recv())
        };

        match notification {
            NetworkNotification::Disconnected => self.state = MqttState::Disconnected,
            NetworkNotification::Connected => {
                self.state = MqttState::Connected;
                try!(self.publish0(true));
                try!(self.publish1(true));
                try!(self.publish2(true));
            }
        }

        // NOTE: Reregister only after 'try_recv' is successful. Or else sync_channel
        // Buffer limit isn't working.
        {
            let nw_notification_rx = self.nw_notification_rx.as_ref().unwrap();
            self.poll
                .reregister(nw_notification_rx, NETWORK_NOTIFICATION_CHANNEL, Ready::readable(), PollOpt::edge())
                .expect("Poll Register Error");
        }

        Ok(())
    }

    fn misc(&mut self) -> Result<()> {
        let notification = {
            let misc_rx = self.misc_rx.as_ref().unwrap();
            try!(misc_rx.try_recv())
        };

        // NOTE: Reregister only after 'try_recv' is successful. Or else sync_channel
        // Buffer limit isn't working.
        {
            let misc_rx = self.misc_rx.as_ref().unwrap();
            self.poll.reregister(misc_rx, MISC_CHANNEL, Ready::readable(), PollOpt::edge()).expect("Poll Register Error");
        }

        let nw_request_tx = self.nw_request_tx.as_ref().unwrap();
        match notification {
            MiscNwRequest::Disconnect => try!(nw_request_tx.send(NetworkRequest::Disconnect)),
            MiscNwRequest::Shutdown => try!(nw_request_tx.send(NetworkRequest::Shutdown)),
        }
        Ok(())
    }


    // http://stackoverflow.
    // com/questions/11115364/mqtt-messageid-practical-implementation
    #[inline]
    fn _next_pkid(&mut self) -> PacketIdentifier {
        let PacketIdentifier(mut pkid) = self.last_pkid;
        if pkid == 65535 {
            pkid = 0;
        }
        self.last_pkid = PacketIdentifier(pkid + 1);
        self.last_pkid
    }
}


// @@@@@@@@@@@@~~~~~~~UNIT TESTS (KIND OF)~~~~~~~~~@@@@@@@@@@@@

#[cfg(test)]
mod test {
    #![allow(unused_variables)]
    extern crate env_logger;
    use mqtt::QualityOfService as QoS;
    use connection::MqttState;
    use clientoptions::MqttOptions;
    use mqtt::control::variable_header::PacketIdentifier;
    use super::MqttClient;

    #[test]
    fn publish0_channel_catchup_pending() {
        const PUBLISH_COUNT: u32 = 1000;

        let client_options = MqttOptions::new().set_pub_q_len(1000);
        let mq_client = MqttClient::new(client_options);
        let (mut client, request, nw_request_rx) = mq_client.mock_start().expect("Coudn't start");
        client.poll().unwrap();

        for i in 0..PUBLISH_COUNT {
            let payload = format!("{}. hello rust", i);
            request.publish("test/qos1/disconnect_publish", QoS::Level0, payload.clone().into_bytes()).unwrap();
            let _ = client.publish0(false);
        }
        assert_eq!(PUBLISH_COUNT, client.pub0_channel_pending);

        client.state = MqttState::Connected;
        let _ = client.publish0(true);
        assert_eq!(0, client.pub0_channel_pending);

        client.state = MqttState::Disconnected;
        for i in 0..PUBLISH_COUNT {
            let payload = format!("{}. hello rust", i);
            request.publish("test/qos1/disconnect_publish", QoS::Level0, payload.clone().into_bytes()).unwrap();
            let _ = client.publish0(false);
        }
        assert_eq!(PUBLISH_COUNT, client.pub0_channel_pending);
    }

    #[test]
    fn publish1_channel_catchup_pending() {
        const PUBLISH_COUNT: u32 = 1000;

        let client_options = MqttOptions::new().set_pub_q_len(1000);
        let mq_client = MqttClient::new(client_options);
        let (mut client, request, nw_request_rx) = mq_client.mock_start().expect("Coudn't start");
        client.poll().unwrap();

        for i in 0..PUBLISH_COUNT {
            let payload = format!("{}. hello rust", i);
            request.publish("test/qos1/disconnect_publish", QoS::Level1, payload.clone().into_bytes()).unwrap();
            let _ = client.publish1(false);
        }
        assert_eq!(PUBLISH_COUNT, client.pub1_channel_pending);

        client.state = MqttState::Connected;
        let _ = client.publish1(true);
        assert_eq!(0, client.pub1_channel_pending);

        client.state = MqttState::Disconnected;
        for i in 0..PUBLISH_COUNT {
            let payload = format!("{}. hello rust", i);
            request.publish("test/qos1/disconnect_publish", QoS::Level1, payload.clone().into_bytes()).unwrap();
            let _ = client.publish1(false);
        }
        assert_eq!(PUBLISH_COUNT, client.pub1_channel_pending);
    }

    #[test]
    fn publish2_channel_catchup_pending() {
        const PUBLISH_COUNT: u32 = 1000;

        let client_options = MqttOptions::new().set_pub_q_len(1000);
        let mq_client = MqttClient::new(client_options);
        let (mut client, request, nw_request_rx) = mq_client.mock_start().expect("Coudn't start");
        client.poll().unwrap();

        for i in 0..PUBLISH_COUNT {
            let payload = format!("{}. hello rust", i);
            request.publish("test/qos1/disconnect_publish", QoS::Level2, payload.clone().into_bytes()).unwrap();
            let _ = client.publish2(false);
        }
        assert_eq!(PUBLISH_COUNT, client.pub2_channel_pending);

        client.state = MqttState::Connected;
        let _ = client.publish2(true);
        assert_eq!(0, client.pub2_channel_pending);

        client.state = MqttState::Disconnected;
        for i in 0..PUBLISH_COUNT {
            let payload = format!("{}. hello rust", i);
            request.publish("test/qos1/disconnect_publish", QoS::Level2, payload.clone().into_bytes()).unwrap();
            let _ = client.publish2(false);
        }
        assert_eq!(PUBLISH_COUNT, client.pub2_channel_pending);
    }

    #[test]
    fn next_pkid_roll() {
        let client_options = MqttOptions::new();
        let mut mq_client = MqttClient::new(client_options);

        for i in 0..65536 {
            mq_client._next_pkid();
        }

        assert_eq!(PacketIdentifier(1), mq_client.last_pkid);
    }
}

use crate::error::{ClientError, ConnectError};
use crate::MqttOptions;
use crossbeam_channel;
use futures::{sync::mpsc, Future, Sink};
use mqtt311::{PacketIdentifier, Publish, QoS, Subscribe, Unsubscribe, SubscribeTopic};
use std::sync::Arc;

mod connection;
mod mqttstate;
mod network;
mod prepend;

/// Incoming notifications from the broker
#[derive(Debug)]
pub enum Notification {
    /// Publish message
    Publish(Publish),
    /// Ack for sent QoS1 publish
    PubAck(PacketIdentifier),
    /// Record for sent Qos2 publish
    PubRec(PacketIdentifier),
    /// Relase for received  Qos2 publish
    PubRel(PacketIdentifier),
    /// Complete for sent release message
    PubComp(PacketIdentifier),
    /// Ack for sent subscription
    SubAck(PacketIdentifier),
    /// Dummy for filtering
    None,
}

/// Requests by the client to mqtt event loop. Request are
/// handle one by one
#[derive(Debug)]
pub enum Request {
    /// Mqtt publish
    Publish(Publish),
    /// Mqtt subscribe
    Subscribe(Subscribe),
    /// Mqtt unsubscribe
    Unsubscribe(Unsubscribe),
    /// Mqtt puback
    PubAck(PacketIdentifier),
    /// Mqtt pubrec
    PubRec(PacketIdentifier),
    /// Mqtt pubrel
    PubRel(PacketIdentifier),
    /// Mqtt pubcomp
    PubComp(PacketIdentifier),
    /// Mqtt ping
    Ping,
    /// Mqtt reconnect
    Reconnect(MqttOptions),
    /// Mqtt disconnect
    Disconnect,
    /// Dummy request
    None,
}

/// Commands sent by the client to mqtt event loop. Commands
/// are of higher priority and will be handled before the
/// next [request]
/// 
/// request: enum.Request.html
#[derive(Debug)]
pub enum Command {
    /// Pause network io by disconnecting the connection. Doesn't
    /// auto reconnect even with auto reconnect option set
    Pause,
    /// Resumes network io by reconnecting to the broker
    Resume,
}

pub struct UserHandle {
    request_tx: mpsc::Sender<Request>,
    command_tx: mpsc::Sender<Command>,
    notification_rx: crossbeam_channel::Receiver<Notification>,
}

/// Handle to send requests and commands to the network eventloop
#[derive(Clone)]
pub struct MqttClient {
    request_tx: mpsc::Sender<Request>,
    command_tx: mpsc::Sender<Command>,
    max_packet_size: usize,
}

impl MqttClient {
    /// Starts a new mqtt connection in a thread and returns [mqttclient] 
    /// instance to send requests/commands to the event loop and a crossbeam
    /// channel receiver to receive notifications sent by the event loop.
    /// 
    /// See `select.rs` example
    /// [mqttclient]: struct.MqttClient.html
    pub fn start(opts: MqttOptions) -> Result<(Self, crossbeam_channel::Receiver<Notification>), ConnectError> {
        let max_packet_size = opts.max_packet_size();
        let UserHandle {
            request_tx,
            command_tx,
            notification_rx,
        } = connection::Connection::run(opts)?;

        let client = MqttClient {
            request_tx,
            command_tx,
            max_packet_size,
        };

        Ok((client, notification_rx))
    }

    /// Requests the eventloop for mqtt publish
    pub fn publish<S, V>(&mut self, topic: S, qos: QoS, payload: V) -> Result<(), ClientError>
    where
        S: Into<String>,
        V: Into<Vec<u8>>,
    {
        let payload = payload.into();
        if payload.len() > self.max_packet_size {
            return Err(ClientError::PacketSizeLimitExceeded);
        }

        let publish = Publish {
            dup: false,
            qos,
            retain: false,
            topic_name: topic.into(),
            pkid: None,
            payload: Arc::new(payload),
        };

        let tx = &mut self.request_tx;
        tx.send(Request::Publish(publish)).wait()?;
        Ok(())
    }

    /// Requests the eventloop for mqtt subscribe
    pub fn subscribe<S>(&mut self, topic: S, qos: QoS) -> Result<(), ClientError>
    where
        S: Into<String>,
    {
        let topic = SubscribeTopic {
            topic_path: topic.into(),
            qos,
        };
        let subscribe = Subscribe {
            pkid: PacketIdentifier::zero(),
            topics: vec![topic],
        };

        let tx = &mut self.request_tx;
        tx.send(Request::Subscribe(subscribe)).wait()?;
        Ok(())
    }

    /// Requests the eventloop for mqtt unsubscribe
    pub fn unsubscribe<S>(&mut self, topic: S) -> Result<(), ClientError>
        where
            S: Into<String>,
    {
        let unsubscribe = Unsubscribe {
            pkid: PacketIdentifier::zero(),
            topics: vec![topic.into()],
        };

        let tx = &mut self.request_tx;
        tx.send(Request::Unsubscribe(unsubscribe)).wait()?;
        Ok(())
    }

    /// Commands the network eventloop to disconnect from the broker.
    /// ReconnectOptions are not in affect here. [Resume] the
    /// network for reconnection
    /// 
    /// [Resume]: struct.MqttClient.html#method.resume 
    pub fn pause(&mut self) -> Result<(), ClientError> {
        let tx = &mut self.command_tx;
        tx.send(Command::Pause).wait()?;
        Ok(())
    }

    /// Commands the network eventloop to reconnect to the broker and
    /// resume network io
    pub fn resume(&mut self) -> Result<(), ClientError> {
        let tx = &mut self.command_tx;
        tx.send(Command::Resume).wait()?;
        Ok(())
    }

    /// Requests the event loop for disconnection
    pub fn disconnect(&mut self) -> Result<(), ClientError> {
        let tx = &mut self.request_tx;
        tx.send(Request::Disconnect).wait()?;
        Ok(())
    }
}

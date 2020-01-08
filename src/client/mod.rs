//! Structs to interact with mqtt eventloop
use crate::{
    error::{ClientError, ConnectError},
    MqttOptions,
};
use futures::{channel::mpsc, sink::SinkExt};
use mqtt311::{PacketIdentifier, Publish, QoS, Subscribe, SubscribeTopic, Unsubscribe};
use std::sync::Arc;

#[doc(hidden)]
pub mod connection;
#[doc(hidden)]
pub mod mqttstate;
#[doc(hidden)]
pub mod network;
#[doc(hidden)]
pub mod prepend;

/// Incoming notifications from the broker
#[derive(Debug)]
pub enum Notification {
    Reconnection,
    Disconnection,
    Publish(Publish),
    PubAck(PacketIdentifier),
    PubRec(PacketIdentifier),
    PubRel(PacketIdentifier),
    PubComp(PacketIdentifier),
    SubAck(PacketIdentifier),
    None,
}

#[doc(hidden)]
/// Requests by the client to mqtt event loop. Request are
/// handle one by one#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Request {
    Publish(Publish),
    Subscribe(Subscribe),
    Unsubscribe(Unsubscribe),
    PubAck(PacketIdentifier),
    PubRec(PacketIdentifier),
    PubRel(PacketIdentifier),
    PubComp(PacketIdentifier),
    IncomingIdlePing,
    OutgoingIdlePing,
    Reconnect(MqttOptions),
    Disconnect,
    None,
}

#[doc(hidden)]
/// Commands sent by the client to mqtt event loop. Commands
/// are of higher priority and will be `select`ed along with
/// [request]s
///
/// request: enum.Request.html
#[derive(Debug)]
pub enum Command {
    Pause,
    Resume,
}

#[doc(hidden)]
/// Combines handles returned by the eventloop
pub struct UserHandle {
    request_tx: mpsc::Sender<Request>,
    command_tx: mpsc::Sender<Command>,
    notification_rx: mpsc::Receiver<Notification>,
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
    /// instance to send requests/commands to the event loop and a
    /// channel receiver to receive notifications sent by the event loop.
    ///
    /// See `select.rs` example
    /// [mqttclient]: struct.MqttClient.html
    pub async fn start(opts: MqttOptions) -> Result<(Self, mpsc::Receiver<Notification>), ConnectError> {
        let max_packet_size = opts.max_packet_size();
        let UserHandle {
            request_tx,
            command_tx,
            notification_rx,
        } = connection::Connection::run(opts).await?;

        let client = MqttClient {
            request_tx,
            command_tx,
            max_packet_size,
        };

        Ok((client, notification_rx))
    }

    /// Requests the eventloop for mqtt publish
    pub async fn publish<S, V, B>(&mut self, topic: S, qos: QoS, retained: B, payload: V) -> Result<(), ClientError>
    where
        S: Into<String>,
        V: Into<Vec<u8>>,
        B: Into<bool>,
    {
        let payload = payload.into();
        if payload.len() > self.max_packet_size {
            return Err(ClientError::PacketSizeLimitExceeded);
        }

        let publish = Publish {
            dup: false,
            qos,
            retain: retained.into(),
            topic_name: topic.into(),
            pkid: None,
            payload: Arc::new(payload),
        };

        self.request_tx.send(Request::Publish(publish))
            .await
            .map_err(ClientError::MpscRequestSend)
    }

    /// Requests the eventloop for mqtt subscribe
    pub async fn subscribe<S>(&mut self, topic: S, qos: QoS) -> Result<(), ClientError>
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

        self.request_tx.send(Request::Subscribe(subscribe))
            .await
            .map_err(ClientError::MpscRequestSend)
    }

    /// Requests the eventloop for mqtt unsubscribe
    pub async fn unsubscribe<S>(&mut self, topic: S) -> Result<(), ClientError>
    where
        S: Into<String>,
    {
        let unsubscribe = Unsubscribe {
            pkid: PacketIdentifier::zero(),
            topics: vec![topic.into()],
        };

        let tx = &mut self.request_tx;
        tx.send(Request::Unsubscribe(unsubscribe))
            .await
            .map_err(ClientError::MpscRequestSend)?;
        Ok(())
    }

    /// Commands the network eventloop to disconnect from the broker.
    /// ReconnectOptions are not in affect here. [Resume] the
    /// network for reconnection
    ///
    /// [Resume]: struct.MqttClient.html#method.resume
    pub async fn pause(&mut self) -> Result<(), ClientError> {
        let tx = &mut self.command_tx;
        tx.send(Command::Pause).await.map_err(ClientError::MpscCommandSend)?;
        Ok(())
    }

    /// Commands the network eventloop to reconnect to the broker and
    /// resume network io
    pub async fn resume(&mut self) -> Result<(), ClientError> {
        let tx = &mut self.command_tx;
        tx.send(Command::Resume).await.map_err(ClientError::MpscCommandSend)?;
        Ok(())
    }

    /// Commands the network eventloop to gracefully shutdown
    /// the connection to the broker.
    pub async fn shutdown(&mut self) -> Result<(), ClientError> {
        let tx = &mut self.request_tx;
        tx.send(Request::Disconnect).await.map_err(ClientError::MpscCommandSend)?;
        Ok(())
    }
}

// use std::fmt;

// impl fmt::Debug for Request {
//     fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
//         match self {
//             Request::Publish(p) => write!(f,
//                 "Publish \
//                  topic = {}, \
//                  pkid = {:?}, \
//                  payload size = {:?} bytes",
//                 p.topic_name,
//                 p.pkid,
//                 p.payload.len()
//             ),

//             _ => write!(f, "{:?}", self),
//         }
//     }
// }

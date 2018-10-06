use std::str;
use std::sync::Arc;
use std::thread;
use std::sync::mpsc::{sync_channel, SyncSender};

use mqtt311::TopicPath;

use error::{Result, Error};
use clientoptions::MqttOptions;
use publisher::{Publisher, PublishRequest};
use callback::{MqttCallback, Message};
use std::time::Duration;
use std::sync::mpsc::TrySendError;

pub struct MqttClient {
    nw_request_tx: SyncSender<PublishRequest>,
}

impl MqttClient {
    /// Connects to the broker and starts an event loop in a new thread.
    /// Returns 'Request' and handles reqests from it.
    /// Also handles network events, reconnections and retransmissions.
    pub fn start(opts: MqttOptions, callbacks: Option<MqttCallback>) -> Result<Self> {
        let (nw_request_tx, nw_request_rx) = sync_channel::<PublishRequest>(1);
        let mut connection = Publisher::connect(opts.clone(), nw_request_rx, callbacks)?;

        // This thread handles network reads (coz they are blocking) and
        // and sends them to event loop thread to handle mqtt state.
        let network_thread = thread::Builder::new().name("rumqtt network".into());
        network_thread.spawn(
            move || -> Result<()> {
                let o = connection.run();
                error!("Network Thread Stopped !!!!!!!!!. Result = {:?}", o);
                Ok(())
            }
        ).expect("Network thread spawn failed");

        let client = MqttClient { nw_request_tx: nw_request_tx };

        Ok(client)
    }

    pub fn publish(&mut self, topic: &str, payload: Vec<u8>) -> Result<()> {
        let payload = Arc::new(payload);
        let mut ret_val;
        loop {
            let payload = payload.clone();
            ret_val = self._publish(topic, payload, None);
            if let Err(Error::TrySend(ref e)) = ret_val {
                match e {
                    // break immediately if rx is dropped
                    &TrySendError::Disconnected(_) => return Err(Error::NoConnectionThread),
                    &TrySendError::Full(_) => {
                        info!("Request Queue Full !!!!!!!!");
                        thread::sleep(Duration::new(3, 0));
                        continue;
                    }
                }
            } else {
                return ret_val;
            }
        }
    }

    pub fn userdata_publish(&mut self, topic: &str, payload: Vec<u8>, userdata: Vec<u8>) -> Result<()> {
        let payload = Arc::new(payload);
        let userdata = Arc::new(userdata);
        let mut ret_val;
        loop {
            let payload = payload.clone();
            ret_val = self._publish(topic, payload, Some(userdata.clone()));
            if let Err(Error::TrySend(ref e)) = ret_val {
                match e {
                    // break immediately if rx is dropped
                    &TrySendError::Disconnected(_) => return Err(Error::NoConnectionThread),
                    &TrySendError::Full(_) => {
                        info!("Request Queue Full !!!!!!!!");
                        thread::sleep(Duration::new(3, 0));
                        continue;
                    }
                }
            } else {
                return ret_val;
            }
        }
    }

    pub fn disconnect(&self) -> Result<()> {
        self.nw_request_tx.try_send(PublishRequest::Disconnect)?;
        Ok(())
    }

    fn _publish(&mut self, topic: &str, payload: Arc<Vec<u8>>, userdata: Option<Arc<Vec<u8>>>) -> Result<()> {

        let _ = TopicPath::from_str(topic.to_string())?;

        let publish = Message {
            topic: topic.to_string(),
            payload: payload,
            pkid: None,
            userdata: userdata,
        };
        let publish = Box::new(publish);

        self.nw_request_tx.try_send(PublishRequest::Publish(publish))?;

        Ok(())
    }
}

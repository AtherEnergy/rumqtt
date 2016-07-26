use std::sync::mpsc::{SyncSender};

use error::{Result};
use mio::*;
use mqtt::{QualityOfService, TopicFilter};

use message::Message;
use client::{MioNotification};

pub type SendableFn = Box<Fn(Message) + Send + Sync>;
pub struct Subscriber {
    pub subscribe_tx: SyncSender<Vec<(TopicFilter, QualityOfService)>>,
    pub mionotify_tx: Sender<MioNotification>,
}

impl Subscriber {
    // TODO Add peek function

    pub fn subscribe(&self, topics: Vec<(&str, QualityOfService)>) -> Result<()> {
        let mut sub_topics = vec![];
        for topic in topics {
            let topic = (try!(TopicFilter::new_checked(topic.0)), topic.1);
            sub_topics.push(topic);
        }

        try!(self.subscribe_tx.send(sub_topics));
        try!(self.mionotify_tx.send(MioNotification::Sub));
        Ok(())
    }
}
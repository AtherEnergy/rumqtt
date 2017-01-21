use mqtt3::{Packet, Protocol, Connect};
use error::*;

use rand::{self, Rng};

pub fn generate_connect_packet(client_id: String,
                               clean_session: bool,
                               username: Option<String>,
                               password: Option<String>)
                               -> Packet {

    let client_id = if client_id == "".to_string() {
        let mut rng = rand::thread_rng();
        let id = rng.gen::<u32>();
        format!("rumqtt_{}", id)
    } else {
        client_id
    };

    Packet::Connect(Box::new(Connect {
        protocol: Protocol::MQTT(4),
        keep_alive: 10,
        client_id: client_id,
        clean_session: clean_session,
        last_will: None,
        username: username,
        password: password,
    }))
}

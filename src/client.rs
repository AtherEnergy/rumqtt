use std::collections::{HashMap, VecDeque};
use std::io::{Write, ErrorKind};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};
use {ReconnectMethod, ClientState};
use std::thread;
use rand::{self, Rng};
use error::{Error, Result};

// #[derive(Clone)]
pub struct ClientOptions {
    keep_alive: Option<Duration>,
    clean_session: bool,
    client_id: Option<String>,
    username: Option<String>,
    password: Option<String>,
    reconnect: ReconnectMethod,
}

pub struct Client {
    addr: SocketAddr,
    state: ClientState,
    // netopt: NetworkOptions,
    opts: ClientOptions,
    // conn: Connection,
    session_present: bool,

    // Queues
    last_flush: Instant,
    await_ping: bool,
}

impl ClientOptions {
    pub fn new() -> ClientOptions {
        ClientOptions {
            keep_alive: Some(Duration::new(30, 0)),
            clean_session: true,
            client_id: None,
            username: None,
            password: None,
            reconnect: ReconnectMethod::ForeverDisconnect,
        }
    }

    pub fn set_keep_alive(&mut self, secs: u16) -> &mut ClientOptions {
        self.keep_alive = Some(Duration::new(secs as u64, 0));
        self
    }

    pub fn set_client_id(&mut self, client_id: String) -> &mut ClientOptions {
        self.client_id = Some(client_id);
        self
    }

    pub fn set_clean_session(&mut self, clean_session: bool) -> &mut ClientOptions {
        self.clean_session = clean_session;
        self
    }


    pub fn generate_client_id(&mut self) -> &mut ClientOptions {
        let mut rng = rand::thread_rng();
        let id = rng.gen::<u32>();
        self.client_id = Some(format!("mqttc_{}", id));
        self
    }

    pub fn set_username(&mut self, username: String) -> &mut ClientOptions {
        self.username = Some(username);
        self
    }

    pub fn set_password(&mut self, password: String) -> &mut ClientOptions {
        self.password = Some(password);
        self
    }

    pub fn set_reconnect(&mut self, reconnect: ReconnectMethod) -> &mut ClientOptions {
        self.reconnect = reconnect;
        self
    }

    pub fn connect<A: ToSocketAddrs>(mut self, addr: A) -> Result<Client> {
        if self.client_id == None {
            self.generate_client_id();
        }

        let addr = try!(addr.to_socket_addrs()).next().expect("Socket address is broken");

        let mut client = Client {
            addr: addr,
            state: ClientState::Disconnected,
            // netopt: netopt,
            opts: self,
            // conn: conn,
            session_present: false,

            // Queues
            last_flush: Instant::now(),
            await_ping: false,
        };

        Ok(client)
    }
}

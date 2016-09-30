use std::net::{TcpStream, SocketAddr, Shutdown};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};
use std::thread;
use std::io::{Write, ErrorKind};
use std::sync::{Arc, RwLock};

use mqtt::packet::*;
use mqtt::Decodable;
use mqtt::control::variable_header::ConnectReturnCode;
use mqtt::control::fixed_header::FixedHeaderError;
use mio::channel::SyncSender;

use error::{Result, Error};
use clientoptions::MqttOptions;
use tls::{NetworkStream, SslContext};
use genpack;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttState {
    Handshake,
    Connected,
    Disconnected,
}

#[derive(Debug)]
pub enum NetworkRequest {
    Shutdown,
    Write(Vec<u8>),
}

pub struct Connection {
    pub addr: SocketAddr,
    pub opts: MqttOptions,
    pub stream: NetworkStream,
    pub incoming_tx: SyncSender<VariablePacket>,
    pub outgoing_rx: Receiver<NetworkRequest>,
    pub state: Arc<RwLock<MqttState>>,
    pub initial_connect: bool,
    pub last_flush: Instant,
}

impl Connection {
    pub fn start(addr: SocketAddr,
                 opts: MqttOptions,
                 state: Arc<RwLock<MqttState>>,
                 incoming_tx: SyncSender<VariablePacket>,
                 outgoing_rx: Receiver<NetworkRequest>)
                 -> Result<Self> {
        
        let mut connection = Connection {
            addr: addr,
            opts: opts,
            stream: NetworkStream::None,
            incoming_tx: incoming_tx,
            outgoing_rx: outgoing_rx,
            state: state,
            initial_connect: true,
            last_flush: Instant::now(),
        };
        try!(connection._try_reconnect());
        try!(connection.stream.set_read_timeout(Some(Duration::new(1, 0))));
        try!(connection._await_connack());
        let mut state = *connection.state.write().unwrap();
        state = MqttState::Connected;
        Ok(connection)
    }

    pub fn run(&mut self) -> Result<()> {
        'reconnect: loop {
            loop {
                if self.initial_connect {
                    self.initial_connect = false;
                    break;
                } else {
                    match self._try_reconnect() {
                        Ok(_) => {
                            try!(self.stream.set_read_timeout(Some(Duration::new(1, 0))));
                            let packet = try!(self._await_connack());
                            try!(self.send_incoming(packet));
                            {
                                let mut state = self.state.write().unwrap();
                                *state = MqttState::Connected;
                            }
                            break;
                        }
                        Err(_) => {
                            continue;
                        }
                    }
                }
            }

            'receive: loop {
                let packet = match VariablePacket::decode(&mut self.stream) {
                    // @ Decoded packet successfully.
                    Ok(pk) => pk,
                    Err(err) => {
                        match err {
                            VariablePacketError::FixedHeaderError(err) => {
                                if let FixedHeaderError::IoError(err) = err {
                                    match err.kind() {
                                        ErrorKind::TimedOut | ErrorKind::WouldBlock => {
                                            let _ = self.write();
                                            if let Err(e) = self.ping() {
                                                match e {
                                                    Error::Timeout => {
                                                        error!("Couldn't PING in time :( . Err = {:?}", e);
                                                        self._unbind();
                                                        continue 'reconnect;
                                                    }
                                                    _ => {
                                                        error!("Error PINGING . Err = {:?}", e);
                                                        self._unbind();
                                                        continue 'reconnect;
                                                    }
                                                }
                                            }
                                            continue 'receive;
                                        }
                                        _ => {
                                            // Socket error are readily available here as soon as
                                            // disconnection happens. So it might be right for this
                                            // thread to ask for reconnection rather than reconnecting
                                            // during write failures
                                            // UPDATE: Lot of publishes are being written by the time this notified
                                            // the eventloop thread. Setting disconnect_block = true during write failure
                                            error!("Error in receiving packet {:?}", err);
                                            self._unbind();
                                            continue 'reconnect;
                                        }
                                    }
                                } else {
                                    error!("Error reading packet = {:?}", err);
                                    self._unbind();
                                    continue 'reconnect;
                                }
                            }
                            _ => {
                                error!("Error reading packet = {:?}", err);
                                self._unbind();
                                continue 'reconnect;
                            }
                        }
                    }
                };
                try!(self.send_incoming(packet))
            }
        }
    }

    fn ping(&mut self) -> Result<()> {
        // debug!("client state --> {:?}, await_ping --> {}", self.state,
        // self.await_ping);
        let state = *self.state.read().unwrap();
        match state {
            MqttState::Connected => {
                if let Some(keep_alive) = self.opts.keep_alive {
                    let elapsed = self.last_flush.elapsed();
                    if elapsed >= Duration::new(keep_alive as u64, 0) {
                        return Err(Error::Timeout);
                    } else if elapsed >= Duration::new((keep_alive as f32 * 0.8) as u64, 0) {
                        try!(self._ping());
                    }
                }
            }

            MqttState::Disconnected | MqttState::Handshake => error!("I won't ping. Client is in disconnected/handshake state"),
        }
        Ok(())
    }

    fn send_incoming(&mut self, packet: VariablePacket) -> Result<()> {
        match self.incoming_tx.send(packet) {
            Ok(_) => (),
            Err(e) => {
                error!("Read Return. Unable to send incoming packets to eventloop. Error = {:?}", e);
                return Err(e.into());
            }
        };
        Ok(())
    }

    fn write(&mut self) -> Result<()> {
        for _ in 0..1000 {
            match try!(self.outgoing_rx.try_recv()) {
                NetworkRequest::Shutdown => try!(self.stream.shutdown(Shutdown::Both)),
                NetworkRequest::Write(packet) => {
                    match self._write_packet(packet) {
                        Ok(..) => (),
                        Err(e) => {
                            error!("Error Writing Packet to Network");
                            return Err(e.into());
                        }
                    }
                }
            };
        }
        Ok(())
    }

    fn _try_reconnect(&mut self) -> Result<()> {
        match self.opts.reconnect {
            // TODO: Implement
            None => panic!("To be implemented"),
            Some(dur) => {
                if !self.initial_connect {
                    error!("  Will try Reconnect in {} seconds", dur);
                    thread::sleep(Duration::new(dur as u64, 0));
                }
                let stream = try!(TcpStream::connect(&self.addr));
                let stream = NetworkStream::Tcp(stream);

                self.stream = stream;
                try!(self._connect());
                {
                    // TODO: Change states properly in one location
                    let mut state = self.state.write().unwrap();
                    *state = MqttState::Connected;
                }
                Ok(())
            }
        }
    }

    fn _await_connack(&mut self) -> Result<VariablePacket> {
        let packet = match VariablePacket::decode(&mut self.stream) {
            Ok(pk) => pk,
            Err(err) => {
                error!("Couldn't decode incoming packet. Error = {:?}", err);
                return Err(Error::InvalidPacket);
            }
        };

        let state = self.state.read().unwrap();
        match *state {
            MqttState::Handshake => {
                match packet {
                    VariablePacket::ConnackPacket(ref connack) => {
                        let conn_ret_code = connack.connect_return_code();
                        if conn_ret_code != ConnectReturnCode::ConnectionAccepted {
                            error!("Failed to connect, err {:?}", conn_ret_code);
                            return Err(Error::ConnectionRefused(conn_ret_code));
                        }
                    }
                    _ => {
                        error!("received invalid packet in handshake state --> {:?}", packet);
                        return Err(Error::InvalidPacket);
                    }
                }
                return Ok(packet);
            }

            MqttState::Disconnected | MqttState::Connected => {
                error!("Invalid State during CONNACK packet");
                return Err(Error::InvalidState);
            }
        }
    }

    fn _connect(&mut self) -> Result<()> {
        let connect = try!(genpack::generate_connect_packet(self.opts.client_id.clone(),
                                                            self.opts.clean_session,
                                                            self.opts.keep_alive,
                                                            self.opts.will.clone(),
                                                            self.opts.will_qos,
                                                            self.opts.will_retain,
                                                            self.opts.username.clone(),
                                                            self.opts.password.clone()));
        try!(self._write_packet(connect));
        self._flush()
    }

    fn _ping(&mut self) -> Result<()> {
        let ping = try!(genpack::generate_pingreq_packet());
        // self.await_ping = true;
        try!(self._write_packet(ping));
        self._flush()
    }

    fn _unbind(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
        // self.await_ping = false;
        {
            let mut state = self.state.write().unwrap();
            *state = MqttState::Disconnected;
        }
        info!("  Disconnected {:?}", self.opts.client_id);
    }

    #[inline]
    fn _write_packet(&mut self, packet: Vec<u8>) -> Result<()> {
        match self.stream.write_all(&packet) {
            Ok(v) => v,
            Err(e) => {
                error!("Error writing packet. Error = {:?}", e);
                return Err(e.into());
            }
        };
        try!(self._flush());
        Ok(())
    }

    fn _flush(&mut self) -> Result<()> {
        try!(self.stream.flush());
        self.last_flush = Instant::now();
        Ok(())
    }
}

//! The real network behind a session: one UDP socket with a reader thread
//! and one event-queue thread per circuit, all feeding the main loop's
//! channel with events stamped with the circuit's generation.

use crate::caps::{self, CapsEvent};
use crate::names::Names;
use crate::session::Net;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use uuid::Uuid;

/// How often a blocked reader thread checks whether it should stop.
const READ_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_DATAGRAM: usize = 65_536;

pub enum Event {
    Datagram { generation: u64, data: Vec<u8> },
    Caps { generation: u64, event: CapsEvent },
    /// The answer to a name lookup: the avatars found, and the ids the grid
    /// does not know as avatars.
    Names { found: Vec<(Uuid, Names)>, missing: Vec<Uuid> },
    /// A name lookup that got no answer.
    NamesFailed(Vec<Uuid>),
    /// The operator asked the bot to stop.
    Signal,
}

struct Link {
    socket: Arc<UdpSocket>,
    stop: Arc<AtomicBool>,
}

pub struct RealNet {
    events: Sender<Event>,
    links: HashMap<u64, Link>,
}

impl RealNet {
    pub fn new(events: Sender<Event>) -> RealNet {
        RealNet { events, links: HashMap::new() }
    }
}

fn connect(addr: SocketAddr) -> std::io::Result<UdpSocket> {
    let local = if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let socket = UdpSocket::bind(local)?;
    socket.connect(addr)?;
    socket.set_read_timeout(Some(READ_TIMEOUT))?;
    Ok(socket)
}

fn read_datagrams(socket: &UdpSocket, generation: u64, stop: &AtomicBool, events: &Sender<Event>) {
    let mut buffer = vec![0; MAX_DATAGRAM];
    while !stop.load(Ordering::Relaxed) {
        match socket.recv(&mut buffer) {
            Ok(len) => {
                let data = buffer[..len].to_vec();
                if events.send(Event::Datagram { generation, data }).is_err() {
                    return;
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => {
                // Typically "connection refused" relayed from an ICMP error;
                // the session notices a dead simulator by its silence.
                crate::debug!("udp receive: {e}");
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

impl Net for RealNet {
    fn open(&mut self, generation: u64, addr: SocketAddr, seed_capability: &str) {
        let socket = match connect(addr) {
            Ok(socket) => Arc::new(socket),
            Err(e) => {
                // Without a socket the handshake times out and the session
                // is reported lost.
                crate::info!("cannot open a UDP socket to {addr}: {e}");
                return;
            }
        };
        let stop = Arc::new(AtomicBool::new(false));

        let (reader_socket, reader_stop, reader_events) = (socket.clone(), stop.clone(), self.events.clone());
        thread::spawn(move || read_datagrams(&reader_socket, generation, &reader_stop, &reader_events));

        let (seed, queue_stop, queue_events) = (seed_capability.to_string(), stop.clone(), self.events.clone());
        thread::spawn(move || {
            caps::run(&seed, &queue_stop, |event| {
                queue_events.send(Event::Caps { generation, event }).is_ok()
            })
        });

        self.links.insert(generation, Link { socket, stop });
    }

    fn send(&mut self, generation: u64, datagram: &[u8]) {
        if let Some(link) = self.links.get(&generation) {
            if let Err(e) = link.socket.send(datagram) {
                crate::debug!("udp send: {e}");
            }
        }
    }

    fn close(&mut self, generation: u64) {
        if let Some(link) = self.links.remove(&generation) {
            link.stop.store(true, Ordering::Relaxed);
        }
    }
}

impl Drop for RealNet {
    fn drop(&mut self) {
        for link in self.links.values() {
            link.stop.store(true, Ordering::Relaxed);
        }
    }
}

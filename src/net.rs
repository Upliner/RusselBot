//! The real network behind a session: one UDP socket with one reader thread
//! for all its circuits, and one event-queue thread per circuit, all feeding
//! the main loop's channel with events stamped with the circuit's generation.

use crate::caps::{self, CapsEvent};
use crate::control::{Request, Response};
use crate::names::Names;
use crate::session::Net;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

/// How often a blocked reader thread checks whether it should stop.
const READ_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_DATAGRAM: usize = 65_536;

pub enum Event {
    Datagram {
        generation: u64,
        data: Vec<u8>,
    },
    Caps {
        generation: u64,
        event: CapsEvent,
    },
    /// The answer to a name lookup: the avatars found, and the ids the grid
    /// does not know as avatars.
    Names {
        found: Vec<(Uuid, Names)>,
        missing: Vec<Uuid>,
    },
    /// A name lookup that got no answer.
    NamesFailed(Vec<Uuid>),
    /// The answer to an avatar search over HTTP: the avatars found, as id
    /// and `First Last`.
    AvatarsFound {
        query: Uuid,
        avatars: Vec<(Uuid, String)>,
    },
    /// An avatar search that got no answer, and why.
    AvatarSearchFailed {
        query: Uuid,
        reason: String,
    },
    /// The answer of the profile service about an avatar.
    Profile {
        id: Uuid,
        result: Result<crate::profile::Profile, String>,
    },
    /// What became of an inventory job.
    Inventory {
        id: u64,
        generation: u64,
        result: Result<crate::inventory::Outcome, crate::inventory::Failed>,
    },
    /// A command from the control socket, to be answered on `reply`.
    Control {
        request: Request,
        reply: Sender<Response>,
    },
    /// What the XMPP component connection reports.
    Xmpp(crate::xmpp::conn::XmppEvent),
    /// The operator asked the bot to stop.
    Signal,
}

/// The one UDP socket all of a session's circuits share, as a viewer's do:
/// a simulator the avatar returns to must see the address it already knows.
/// A reader thread reports each datagram under the generation of the
/// circuit that owns the address it came from.
struct Udp {
    socket: Arc<UdpSocket>,
    owners: Arc<Mutex<HashMap<SocketAddr, u64>>>,
    stop: Arc<AtomicBool>,
}

impl Udp {
    /// Binds a socket of the address family `peer` has and starts reading.
    fn bind(peer: SocketAddr, events: Sender<Event>) -> std::io::Result<Udp> {
        let local = if peer.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
        let socket = Arc::new(UdpSocket::bind(local)?);
        socket.set_read_timeout(Some(READ_TIMEOUT))?;
        let udp = Udp {
            socket,
            owners: Arc::default(),
            stop: Arc::default(),
        };
        let (socket, owners, stop) = (udp.socket.clone(), udp.owners.clone(), udp.stop.clone());
        thread::spawn(move || read_datagrams(&socket, &owners, &stop, &events));
        Ok(udp)
    }

    /// Datagrams from `addr` belong to `generation` from now on.
    fn claim(&self, addr: SocketAddr, generation: u64) {
        lock(&self.owners).insert(addr, generation);
    }

    /// Datagrams from `addr` are dropped, unless a later circuit has
    /// claimed the address since.
    fn release(&self, addr: SocketAddr, generation: u64) {
        let mut owners = lock(&self.owners);
        if owners.get(&addr) == Some(&generation) {
            owners.remove(&addr);
        }
    }

    fn send(&self, addr: SocketAddr, datagram: &[u8]) {
        if let Err(e) = self.socket.send_to(datagram, addr) {
            crate::debug!("udp send to {addr}: {e}");
        }
    }
}

impl Drop for Udp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn read_datagrams(socket: &UdpSocket, owners: &Mutex<HashMap<SocketAddr, u64>>, stop: &AtomicBool, events: &Sender<Event>) {
    let mut buffer = vec![0; MAX_DATAGRAM];
    while !stop.load(Ordering::Relaxed) {
        match socket.recv_from(&mut buffer) {
            Ok((len, from)) => {
                // Whoever else sends to this port is none of the session's
                // simulators.
                let Some(generation) = lock(owners).get(&from).copied() else {
                    continue;
                };
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

/// One circuit: the simulator it talks to, and the switch that stops its
/// event-queue thread.
struct Link {
    addr: SocketAddr,
    queue_stop: Arc<AtomicBool>,
}

pub struct RealNet {
    events: Sender<Event>,
    /// Bound when the first circuit opens.
    udp: Option<Udp>,
    links: HashMap<u64, Link>,
    /// Whether circuits get an event-queue thread; only tests have none.
    queues: bool,
}

impl RealNet {
    pub fn new(events: Sender<Event>) -> RealNet {
        RealNet {
            events,
            udp: None,
            links: HashMap::new(),
            queues: true,
        }
    }
}

impl Net for RealNet {
    fn open(&mut self, generation: u64, addr: SocketAddr, seed_capability: &str) {
        if self.udp.is_none() {
            match Udp::bind(addr, self.events.clone()) {
                Ok(udp) => self.udp = Some(udp),
                Err(e) => {
                    // Without a socket the handshake times out and the
                    // session is reported lost.
                    crate::info!("cannot open a UDP socket to {addr}: {e}");
                    return;
                }
            }
        }
        // An earlier circuit to the same simulator gives the address up.
        self.links.retain(|_, link| {
            if link.addr == addr {
                link.queue_stop.store(true, Ordering::Relaxed);
            }
            link.addr != addr
        });
        if let Some(udp) = &self.udp {
            udp.claim(addr, generation);
        }

        let queue_stop = Arc::new(AtomicBool::new(false));
        if self.queues {
            let (seed, stop, events) = (seed_capability.to_string(), queue_stop.clone(), self.events.clone());
            thread::spawn(move || caps::run(&seed, &stop, |event| events.send(Event::Caps { generation, event }).is_ok()));
        }
        self.links.insert(generation, Link { addr, queue_stop });
    }

    fn send(&mut self, generation: u64, datagram: &[u8]) {
        if let (Some(udp), Some(link)) = (&self.udp, self.links.get(&generation)) {
            udp.send(link.addr, datagram);
        }
    }

    fn leave(&mut self, generation: u64) {
        if let Some(link) = self.links.get(&generation) {
            link.queue_stop.store(true, Ordering::Relaxed);
        }
    }

    fn close(&mut self, generation: u64) {
        if let Some(link) = self.links.remove(&generation) {
            link.queue_stop.store(true, Ordering::Relaxed);
            if let Some(udp) = &self.udp {
                udp.release(link.addr, generation);
            }
        }
    }
}

impl Drop for RealNet {
    fn drop(&mut self) {
        for link in self.links.values() {
            link.queue_stop.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Receiver};

    /// A socket standing in for a simulator.
    struct Sim {
        socket: UdpSocket,
        addr: SocketAddr,
    }

    impl Sim {
        fn new() -> Sim {
            let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
            socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let addr = socket.local_addr().unwrap();
            Sim { socket, addr }
        }

        /// The next datagram and where it came from.
        fn receive(&self) -> (Vec<u8>, SocketAddr) {
            let mut buffer = [0; 64];
            let (len, from) = self.socket.recv_from(&mut buffer).unwrap();
            (buffer[..len].to_vec(), from)
        }
    }

    fn net() -> (RealNet, Receiver<Event>) {
        let (events, received) = channel();
        let mut net = RealNet::new(events);
        net.queues = false;
        (net, received)
    }

    /// The next datagram reported, with its generation.
    fn datagram(received: &Receiver<Event>) -> (u64, Vec<u8>) {
        match received.recv_timeout(Duration::from_secs(5)) {
            Ok(Event::Datagram { generation, data }) => (generation, data),
            Ok(_) => panic!("not a datagram"),
            Err(e) => panic!("no datagram: {e}"),
        }
    }

    #[test]
    fn all_circuits_send_from_one_port_and_answers_find_their_circuit() {
        let (home, target, stranger) = (Sim::new(), Sim::new(), Sim::new());
        let (mut net, received) = net();
        net.open(1, home.addr, "");
        net.open(2, target.addr, "");

        net.send(1, b"to home");
        net.send(2, b"to target");
        let (data, bot) = home.receive();
        assert_eq!(data, b"to home");
        let (data, from) = target.receive();
        assert_eq!(data, b"to target");
        assert_eq!(from, bot, "both simulators see one address");

        // A stranger is not answered and not reported: what comes next is
        // what the simulators sent afterwards.
        stranger.socket.send_to(b"who is there", bot).unwrap();
        target.socket.send_to(b"from target", bot).unwrap();
        assert_eq!(datagram(&received), (2, b"from target".to_vec()));
        home.socket.send_to(b"from home", bot).unwrap();
        assert_eq!(datagram(&received), (1, b"from home".to_vec()));
    }

    #[test]
    fn a_later_circuit_to_the_same_simulator_takes_its_address_over() {
        let (home, target) = (Sim::new(), Sim::new());
        let (mut net, received) = net();
        net.open(1, home.addr, "");
        net.send(1, b"first");
        let (_, bot) = home.receive();
        net.leave(1);
        net.open(2, target.addr, "");

        // Left, the circuit still sends and receives.
        net.send(1, b"still here");
        assert_eq!(home.receive(), (b"still here".to_vec(), bot));
        home.socket.send_to(b"ping", bot).unwrap();
        assert_eq!(datagram(&received), (1, b"ping".to_vec()));

        net.leave(2);
        net.open(3, home.addr, "");
        net.send(3, b"back");
        assert_eq!(home.receive(), (b"back".to_vec(), bot), "from the port the simulator knows");
        home.socket.send_to(b"welcome back", bot).unwrap();
        assert_eq!(datagram(&received), (3, b"welcome back".to_vec()));

        // The superseded circuit is gone: it sends nothing, and closing it
        // does not take the address from its successor.
        net.send(1, b"lost");
        net.close(1);
        net.send(3, b"again");
        assert_eq!(home.receive().0, b"again");
        home.socket.send_to(b"still yours", bot).unwrap();
        assert_eq!(datagram(&received), (3, b"still yours".to_vec()));
    }

    #[test]
    fn a_closed_circuit_reports_nothing_more() {
        let (home, target) = (Sim::new(), Sim::new());
        let (mut net, received) = net();
        net.open(1, home.addr, "");
        net.open(2, target.addr, "");
        net.send(1, b"hello");
        let (_, bot) = home.receive();

        net.close(1);
        net.send(1, b"into the void");
        home.socket.send_to(b"from the closed one", bot).unwrap();
        target.socket.send_to(b"from the open one", bot).unwrap();
        assert_eq!(datagram(&received), (2, b"from the open one".to_vec()));
        home.socket.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        assert!(home.socket.recv_from(&mut [0; 64]).is_err(), "nothing is sent on a closed circuit");
    }
}

//! Reliability bookkeeping for one simulator circuit: sequence numbers,
//! acks owed to the simulator, resends of our own reliable packets,
//! duplicate suppression and liveness. Pure state; the caller does the I/O.

use super::messages::{self, Outgoing};
use super::packet::{self, Packet, FLAG_RELIABLE, FLAG_RESENT};
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

/// How long acks may be held back to batch them.
pub const ACK_FLUSH_DELAY: Duration = Duration::from_millis(200);
/// Delay before the first resend; doubles after each one.
pub const FIRST_RESEND_DELAY: Duration = Duration::from_secs(1);
/// Total transmissions of a reliable packet before giving up.
pub const MAX_SENDS: u32 = 5;
/// How many recent reliable sequence numbers are remembered.
const SEEN_CAPACITY: usize = 1024;
const MAX_ACKS_PER_PACKET: usize = 255;

struct Unacked {
    sequence: u32,
    datagram: Vec<u8>,
    sends: u32,
    next_action: Instant,
    interval: Duration,
}

pub struct Circuit {
    next_sequence: u32,
    pending_acks: Vec<u32>,
    ack_deadline: Option<Instant>,
    unacked: Vec<Unacked>,
    seen: HashSet<u32>,
    seen_order: VecDeque<u32>,
    last_receive: Instant,
}

/// What `poll` wants done.
#[derive(Debug, Default, PartialEq)]
pub struct Poll {
    pub datagrams: Vec<Vec<u8>>,
    /// A reliable packet went unacknowledged through every resend.
    pub exhausted: bool,
}

impl Circuit {
    pub fn new(now: Instant) -> Circuit {
        Circuit {
            next_sequence: 1,
            pending_acks: Vec::new(),
            ack_deadline: None,
            unacked: Vec::new(),
            seen: HashSet::new(),
            seen_order: VecDeque::new(),
            last_receive: now,
        }
    }

    /// Takes the circuit up again after a time in which nothing on it was
    /// acted on: the avatar is back at a simulator it had left. Our
    /// numbering carries on, because the simulator's client for the avatar
    /// drops numbers it has seen, and so do the acks owed. The silence clock
    /// starts over. So does the duplicate filter: the simulator may have
    /// replaced that client with a new one, which numbers from 1 again. And
    /// what still awaited an ack is dropped: it belonged to the stay that
    /// ended, and a new client would carry out an old request as a new one.
    pub fn resume(&mut self, now: Instant) {
        self.last_receive = now;
        self.seen.clear();
        self.seen_order.clear();
        self.unacked.clear();
    }

    /// Frames a message with the next sequence number. Reliable messages are
    /// kept for resending until acknowledged.
    pub fn send(&mut self, message: &Outgoing, reliable: bool, now: Instant) -> Vec<u8> {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        let flags = if reliable { FLAG_RELIABLE } else { 0 };
        let datagram = packet::build(flags, sequence, message.id, &message.body, message.zerocoded);
        if reliable {
            self.unacked.push(Unacked {
                sequence,
                datagram: datagram.clone(),
                sends: 1,
                next_action: now + FIRST_RESEND_DELAY,
                interval: FIRST_RESEND_DELAY,
            });
        }
        datagram
    }

    /// Records an incoming packet. Returns false when it is a duplicate of a
    /// reliable packet already processed; it is still acknowledged again.
    pub fn receive(&mut self, packet: &Packet, now: Instant) -> bool {
        self.last_receive = now;
        self.acknowledged(&packet.acks);
        if !packet.is_reliable() {
            return true;
        }
        self.pending_acks.push(packet.sequence);
        self.ack_deadline.get_or_insert(now + ACK_FLUSH_DELAY);
        if !self.seen.insert(packet.sequence) {
            return false;
        }
        self.seen_order.push_back(packet.sequence);
        if self.seen_order.len() > SEEN_CAPACITY {
            if let Some(oldest) = self.seen_order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        true
    }

    /// The simulator acknowledged these sequence numbers of ours.
    pub fn acknowledged(&mut self, sequences: &[u32]) {
        if !sequences.is_empty() {
            self.unacked.retain(|u| !sequences.contains(&u.sequence));
        }
    }

    /// Flushes due acks and resends due reliable packets.
    pub fn poll(&mut self, now: Instant) -> Poll {
        let mut poll = Poll::default();
        if self.ack_deadline.is_some_and(|deadline| now >= deadline) {
            self.ack_deadline = None;
            let acks = std::mem::take(&mut self.pending_acks);
            for chunk in acks.chunks(MAX_ACKS_PER_PACKET) {
                let message = messages::packet_ack(chunk);
                poll.datagrams.push(self.send(&message, false, now));
            }
        }
        for unacked in &mut self.unacked {
            if now < unacked.next_action {
                continue;
            }
            if unacked.sends >= MAX_SENDS {
                poll.exhausted = true;
                continue;
            }
            unacked.datagram[0] |= FLAG_RESENT;
            unacked.sends += 1;
            unacked.interval *= 2;
            unacked.next_action = now + unacked.interval;
            poll.datagrams.push(unacked.datagram.clone());
        }
        poll
    }

    /// When `poll` next has something to do.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.unacked.iter().map(|u| u.next_action).chain(self.ack_deadline).min()
    }

    pub fn last_receive(&self) -> Instant {
        self.last_receive
    }

    #[cfg(test)]
    pub fn unacked_count(&self) -> usize {
        self.unacked.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::udp::packet::{high, parse, FLAG_ACKS};

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn incoming(sequence: u32, reliable: bool) -> Packet {
        let flags = if reliable { FLAG_RELIABLE } else { 0 };
        Packet {
            flags,
            sequence,
            acks: Vec::new(),
            id: high(1),
            body: vec![0; 5],
        }
    }

    #[test]
    fn sequence_numbers_increase_from_one() {
        let now = Instant::now();
        let mut circuit = Circuit::new(now);
        let first = circuit.send(&messages::complete_ping_check(1), false, now);
        let second = circuit.send(&messages::complete_ping_check(2), true, now);
        assert_eq!(parse(&first).unwrap().sequence, 1);
        let second = parse(&second).unwrap();
        assert_eq!(second.sequence, 2);
        assert!(second.is_reliable());
        assert_eq!(circuit.unacked_count(), 1);
    }

    #[test]
    fn resends_with_doubling_interval_then_gives_up() {
        let start = Instant::now();
        let mut circuit = Circuit::new(start);
        let original = circuit.send(&messages::complete_ping_check(1), true, start);

        assert_eq!(circuit.poll(start + Duration::from_millis(999)), Poll::default());
        assert_eq!(circuit.next_deadline(), Some(start + secs(1)));

        // Resends at 1 s, then 2, 4 and 8 s after the previous one.
        let mut at = start;
        for gap in [1, 2, 4, 8] {
            at += secs(gap);
            let poll = circuit.poll(at);
            assert!(!poll.exhausted);
            assert_eq!(poll.datagrams.len(), 1, "resend {gap}");
            let resent = &poll.datagrams[0];
            assert_eq!(resent[0], original[0] | FLAG_RESENT);
            assert_eq!(resent[1..], original[1..]);
            assert_eq!(circuit.poll(at), Poll::default());
        }
        // Five sends in total; after the last wait the circuit is declared dead.
        assert_eq!(circuit.poll(at + secs(15)), Poll::default());
        let poll = circuit.poll(at + secs(16));
        assert!(poll.exhausted && poll.datagrams.is_empty());
    }

    #[test]
    fn acknowledged_packets_are_not_resent() {
        let start = Instant::now();
        let mut circuit = Circuit::new(start);
        circuit.send(&messages::complete_ping_check(1), true, start);
        circuit.send(&messages::complete_ping_check(2), true, start);
        circuit.acknowledged(&[1]);
        let poll = circuit.poll(start + secs(1));
        assert_eq!(poll.datagrams.len(), 1);
        assert_eq!(parse(&poll.datagrams[0]).unwrap().sequence, 2);
    }

    #[test]
    fn appended_acks_count_as_acknowledgement() {
        let start = Instant::now();
        let mut circuit = Circuit::new(start);
        circuit.send(&messages::complete_ping_check(1), true, start);
        let mut packet = incoming(10, false);
        packet.flags |= FLAG_ACKS;
        packet.acks = vec![1];
        assert!(circuit.receive(&packet, start));
        assert_eq!(circuit.unacked_count(), 0);
        assert_eq!(circuit.next_deadline(), None);
    }

    #[test]
    fn reliable_packets_are_acked_in_one_batch_after_the_flush_delay() {
        let start = Instant::now();
        let mut circuit = Circuit::new(start);
        assert!(circuit.receive(&incoming(7, true), start));
        assert!(circuit.receive(&incoming(8, true), start + Duration::from_millis(100)));
        assert!(circuit.receive(&incoming(9, false), start + Duration::from_millis(100)));
        assert_eq!(circuit.next_deadline(), Some(start + ACK_FLUSH_DELAY));
        assert_eq!(circuit.poll(start + Duration::from_millis(199)), Poll::default());

        let poll = circuit.poll(start + ACK_FLUSH_DELAY);
        assert_eq!(poll.datagrams.len(), 1);
        let ack = parse(&poll.datagrams[0]).unwrap();
        assert!(!ack.is_reliable());
        assert_eq!(messages::decode(ack.id, &ack.body), Ok(messages::Incoming::PacketAck(vec![7, 8])));
        assert_eq!(circuit.next_deadline(), None);
    }

    #[test]
    fn duplicates_are_acked_again_but_not_processed() {
        let start = Instant::now();
        let mut circuit = Circuit::new(start);
        assert!(circuit.receive(&incoming(7, true), start));
        circuit.poll(start + ACK_FLUSH_DELAY);

        let later = start + secs(2);
        assert!(!circuit.receive(&incoming(7, true), later));
        let poll = circuit.poll(later + ACK_FLUSH_DELAY);
        let ack = parse(&poll.datagrams[0]).unwrap();
        assert_eq!(messages::decode(ack.id, &ack.body), Ok(messages::Incoming::PacketAck(vec![7])));
        // Unreliable packets are never treated as duplicates.
        assert!(circuit.receive(&incoming(7, false), later));
    }

    #[test]
    fn many_acks_are_split_across_packets() {
        let start = Instant::now();
        let mut circuit = Circuit::new(start);
        for sequence in 0..300 {
            circuit.receive(&incoming(sequence, true), start);
        }
        assert_eq!(circuit.poll(start + ACK_FLUSH_DELAY).datagrams.len(), 2);
    }

    #[test]
    fn a_resumed_circuit_keeps_its_numbering_and_the_acks_it_owes() {
        let start = Instant::now();
        let mut circuit = Circuit::new(start);
        circuit.send(&messages::complete_ping_check(1), false, start);
        circuit.send(&messages::complete_ping_check(2), false, start);
        assert!(circuit.receive(&incoming(7, true), start));

        let later = start + secs(300);
        circuit.resume(later);
        assert_eq!(circuit.last_receive(), later, "the silence clock starts over");
        let next = circuit.send(&messages::complete_ping_check(3), false, later);
        assert_eq!(parse(&next).unwrap().sequence, 3);
        let poll = circuit.poll(later);
        let ack = parse(&poll.datagrams[0]).unwrap();
        assert_eq!(messages::decode(ack.id, &ack.body), Ok(messages::Incoming::PacketAck(vec![7])));
    }

    #[test]
    fn a_resumed_circuit_takes_numbers_it_has_seen_and_resends_nothing_old() {
        let start = Instant::now();
        let mut circuit = Circuit::new(start);
        assert!(circuit.receive(&incoming(1, true), start));
        assert!(!circuit.receive(&incoming(1, true), start));
        circuit.send(&messages::complete_ping_check(1), true, start);
        assert_eq!(circuit.unacked_count(), 1);

        circuit.resume(start);
        // The simulator's new client starts at 1 again.
        assert!(circuit.receive(&incoming(1, true), start));
        assert!(!circuit.receive(&incoming(1, true), start));
        // A request of the stay that ended is not repeated to it.
        assert_eq!(circuit.unacked_count(), 0);
        let poll = circuit.poll(start + secs(60));
        assert!(!poll.exhausted);
        assert!(poll.datagrams.iter().all(|datagram| !parse(datagram).unwrap().is_reliable()));
    }

    #[test]
    fn tracks_last_receive_time() {
        let start = Instant::now();
        let mut circuit = Circuit::new(start);
        assert_eq!(circuit.last_receive(), start);
        circuit.receive(&incoming(1, false), start + secs(30));
        assert_eq!(circuit.last_receive(), start + secs(30));
    }
}

#!/usr/bin/env python3
"""Rig helper: makes, ends, hides and shows the friendship between the bot's
account and the estate owner, for the bot's `friends` and `profile`
commands. Stands in for doing the same with a viewer.

  make   logs in as `Sit Bot`, which offers `Rig Owner` friendship, and then
         as `Rig Owner`, which accepts. Run it while the bot is offline: the
         login as `Sit Bot` would end the bot's session.
  end    logs in as `Rig Owner` and ends the friendship.
  hide   logs in as `Rig Owner` and takes away the bot's right to see when
         `Rig Owner` is online.
  show   logs in as `Rig Owner` and gives that right back.

Usage: friend.py make | end | hide | show"""
import hashlib, re, socket, struct, sys, time, urllib.request, uuid

LOGIN = "http://127.0.0.1:9000/"
BOT = ("Sit", "Bot", "secret")
OWNER = ("Rig", "Owner", "rigowner")
# The rights a friend grants: 1 is seeing it online (GRANT_ONLINE_STATUS).
SEE_ONLINE = 1


def member(n, v):
    return "<member><name>%s</name><value><string>%s</string></value></member>" % (n, v)


def low(n):
    return b"\xff\xff" + struct.pack(">H", n)


class Avatar:
    """One login and its circuit, enough to send a few messages."""

    def __init__(self, first, last, password):
        body = ("<?xml version=\"1.0\"?><methodCall><methodName>login_to_simulator</methodName><params><param><value><struct>"
                + member("first", first) + member("last", last)
                + member("passwd", "$1$" + hashlib.md5(password.encode()).hexdigest())
                + member("start", "last") + member("channel", "rig-friend") + member("version", "0.1")
                + member("platform", "Lnx") + member("mac", "0" * 32) + member("id0", "0" * 32)
                + "<member><name>agree_to_tos</name><value><boolean>1</boolean></value></member>"
                + "<member><name>options</name><value><array><data><value><string>buddy-list</string></value>"
                + "</data></array></value></member>"
                + "</struct></value></param></params></methodCall>")
        request = urllib.request.Request(LOGIN, body.encode(), {"Content-Type": "text/xml"})
        self.reply = urllib.request.urlopen(request).read().decode()
        self.name = first + " " + last
        self.id = uuid.UUID(self.field("agent_id"))
        self.agent, self.session = self.id.bytes, uuid.UUID(self.field("session_id")).bytes
        code, self.peer = int(self.field("circuit_code")), (self.field("sim_ip"), int(self.field("sim_port")))
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.settimeout(0.2)
        self.seq = 0
        self.send(low(3), struct.pack("<I", code) + self.session + self.agent)
        self.send(low(249), self.agent + self.session + struct.pack("<I", code))
        arrived, shaken, deadline = False, False, time.time() + 15
        while time.time() < deadline and not (arrived and shaken):
            mid, _ = self.receive()
            if mid == ("L", 148):
                self.send(low(149), self.agent + self.session + bytes(4))
                shaken = True
            elif mid == ("L", 250):
                arrived = True
        if not (arrived and shaken):
            sys.exit("%s: handshake failed" % self.name)
        self.idle(1)

    def field(self, name):
        found = re.findall(r"<name>%s</name>\s*<value>\s*<(?:string|int|i4)>([^<]*)<" % name, self.reply)
        found = [v for v in found if not v.startswith("11111111-1111")]  # the library owner's nested agent_id
        if not found:
            sys.exit("login failed: " + self.reply[:400])
        return found[0]

    def buddies(self):
        """The `buddy_id`s of the login reply's `buddy-list`."""
        return re.findall(r"<name>buddy_id</name>\s*<value>\s*<string>([^<]*)<", self.reply)

    def send(self, mid, payload, reliable=True):
        self.seq += 1
        self.sock.sendto(bytes([0x40 if reliable else 0]) + struct.pack(">IB", self.seq, 0) + mid + payload, self.peer)

    def receive(self):
        try:
            data, _ = self.sock.recvfrom(65536)
        except socket.timeout:
            return None, b""
        flags, rseq, extra = data[0], struct.unpack(">I", data[1:5])[0], data[5]
        payload = data[6 + extra:]
        if flags & 0x10:
            payload = payload[: -1 - 4 * payload[-1]]
        if flags & 0x80:
            out, i = bytearray(), 0
            while i < len(payload):
                if payload[i] == 0:
                    out += bytes(payload[i + 1]); i += 2
                else:
                    out.append(payload[i]); i += 1
            payload = bytes(out)
        if flags & 0x40:
            self.send(b"\xff\xff\xff\xfb", bytes([1]) + struct.pack("<I", rseq), reliable=False)
        if payload[0] != 0xFF:
            return ("H", payload[0]), payload[1:]
        if payload[1] != 0xFF:
            return ("M", payload[1]), payload[2:]
        return ("L", struct.unpack(">H", payload[2:4])[0]), payload[4:]

    def idle(self, seconds):
        """Keeps the circuit alive: acknowledges and answers pings."""
        deadline = time.time() + seconds
        while time.time() < deadline:
            mid, payload = self.receive()
            if mid == ("H", 1):
                self.send(b"\x02", payload[:1], reliable=False)

    def offer_friendship(self, to):
        """ImprovedInstantMessage with dialog 38, FriendshipOffered."""
        name = self.name.encode() + b"\0"
        text = b"Will you be my friend?\0"
        im = self.agent + self.session + bytes([0]) + to.bytes + struct.pack("<I", 0) + bytes(16) + bytes(12)
        im += bytes([0, 38]) + self.agent + struct.pack("<I", 0) + bytes([len(name)]) + name
        im += struct.pack("<H", len(text)) + text + struct.pack("<H", 0) + struct.pack("<I", 0) + bytes([0])
        self.send(low(254), im)
        self.idle(2)

    def accept_friendship(self, offered_by):
        """AcceptFriendship, Low 297: OpenSimulator takes the transaction
        id for the avatar that made the offer."""
        self.send(low(297), self.agent + self.session + offered_by.bytes + bytes([0]))
        self.idle(2)

    def end_friendship(self, other):
        """TerminateFriendship, Low 300."""
        self.send(low(300), self.agent + self.session + other.bytes)
        self.idle(2)

    def grant(self, friend, rights):
        """GrantUserRights, Low 320."""
        self.send(low(320), self.agent + self.session + bytes([1]) + friend.bytes + struct.pack("<i", rights))
        self.idle(2)

    def logout(self):
        self.send(low(252), self.agent + self.session)
        self.idle(1)


if len(sys.argv) != 2 or sys.argv[1] not in ("make", "end", "hide", "show"):
    sys.exit(__doc__)
what = sys.argv[1]
if what == "make":
    # The owner's UUID is the rig's own; a login tells it.
    owner = Avatar(*OWNER)
    owner_id = owner.id
    owner.logout()
    # Offered while the owner is offline: OpenSimulator keeps the offer.
    bot = Avatar(*BOT)
    bot_id = bot.id
    bot.offer_friendship(owner_id)
    bot.logout()
    owner = Avatar(*OWNER)
    owner.accept_friendship(bot_id)
    owner.logout()
    print("Sit Bot <%s> and Rig Owner <%s> are friends" % (bot_id, owner_id))
    check = Avatar(*BOT)
    print("Sit Bot's buddy-list at the next login: %s" % (", ".join(check.buddies()) or "none"))
    check.logout()
else:
    owner = Avatar(*OWNER)
    bot_id = uuid.UUID("7e570000-0000-4000-8000-0000000000b0")
    if what == "end":
        owner.end_friendship(bot_id)
    else:
        owner.grant(bot_id, SEE_ONLINE if what == "show" else 0)
    print("%s: done; Rig Owner's friends at this login: %s" % (what, ", ".join(owner.buddies()) or "none"))
    owner.logout()

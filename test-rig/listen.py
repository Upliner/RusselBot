#!/usr/bin/env python3
"""Rig helper: logs in as the estate owner next to the bot and listens, to
exercise the bot's control socket. For SECONDS it prints every line of local
chat it hears and every instant message it gets, and answers each instant
message from the bot with "pong: <text>". It also prints where the bot's
avatar is and what it is attached to whenever that changes, answers the
bot's request for a teleport with an offer, and prints the offers the bot
sends. Stands in for watching with a viewer. Start it while the bot is
seated in Target, then use `rslbot ctl`.

Usage: listen.py [SECONDS] [prim] [offer=N]   (default: 30 seconds)

  prim      rez a box near the bot with the library script DefaultWithTouch,
            which says "Touched." when clicked, and print its UUID; the box
            goes to the trash at the end
  offer=N   after N seconds, offer the bot a teleport it did not ask for

Output lines:
  chat <type> <name>: <text>          im <name> <id>: <text>
  bot at <x> <y> <z> parent <local id>   (parent 0: not sitting on an object)
  teleport request from <name>: <text>   teleport offer from <name>: <text>
  touch prim <uuid>
Chat on channels other than 0 reaches scripts only, so it never shows here."""
import hashlib, re, socket, struct, sys, time, urllib.request, uuid

FIRST, LAST, PASSWORD = "Rig", "Owner", "rigowner"
LOGIN = "http://127.0.0.1:9000/"
ARGS = sys.argv[1:]
SECONDS = float(next((a for a in ARGS if a.replace(".", "").isdigit()), 30.0))
PRIM = "prim" in ARGS
OFFER_AFTER = next((float(a[6:]) for a in ARGS if a.startswith("offer=")), None)
BOT = uuid.UUID("7e570000-0000-4000-8000-0000000000b0")
TOUCH_SCRIPT = uuid.UUID("9267e689-4f37-4ce5-b2c2-455ee2e1ab12")  # library item DefaultWithTouch


def member(n, v):
    return "<member><name>%s</name><value><string>%s</string></value></member>" % (n, v)


body = ("<?xml version=\"1.0\"?><methodCall><methodName>login_to_simulator</methodName><params><param><value><struct>"
        + member("first", FIRST) + member("last", LAST)
        + member("passwd", "$1$" + hashlib.md5(PASSWORD.encode()).hexdigest())
        + member("start", "uri:Target&amp;124&amp;128&amp;25") + member("channel", "rig-listener") + member("version", "0.1")
        + member("platform", "Lnx") + member("mac", "0" * 32) + member("id0", "0" * 32)
        + "<member><name>agree_to_tos</name><value><boolean>1</boolean></value></member>"
        + "<member><name>options</name><value><array><data></data></array></value></member>"
        + "</struct></value></param></params></methodCall>")
reply = urllib.request.urlopen(urllib.request.Request(LOGIN, body.encode(), {"Content-Type": "text/xml"})).read().decode()


def field(name):
    found = re.findall(r"<name>%s</name>\s*<value>\s*<(?:string|int|i4)>([^<]*)<" % name, reply)
    found = [v for v in found if not v.startswith("11111111-1111")]  # skip the library owner's nested agent_id
    if not found:
        sys.exit("login failed: " + reply[:400])
    return found[0]


agent, session = uuid.UUID(field("agent_id")).bytes, uuid.UUID(field("session_id")).bytes
code, peer = int(field("circuit_code")), (field("sim_ip"), int(field("sim_port")))
sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.settimeout(0.2)
seq = 0


def send(mid, payload, reliable=True):
    global seq
    seq += 1
    sock.sendto(bytes([0x40 if reliable else 0]) + struct.pack(">IB", seq, 0) + mid + payload, peer)


def low(n):
    return b"\xff\xff" + struct.pack(">H", n)


def zdecode(d):
    out, i = bytearray(), 0
    while i < len(d):
        if d[i] == 0:
            out += bytes(d[i + 1]); i += 2
        else:
            out.append(d[i]); i += 1
    return bytes(out)


def receive():
    try:
        data, _ = sock.recvfrom(65536)
    except socket.timeout:
        return None, b""
    flags, rseq, extra = data[0], struct.unpack(">I", data[1:5])[0], data[5]
    payload = data[6 + extra:]
    if flags & 0x10:
        payload = payload[: -1 - 4 * payload[-1]]
    if flags & 0x80:
        payload = zdecode(payload)
    if flags & 0x40:
        send(b"\xff\xff\xff\xfb", bytes([1]) + struct.pack("<I", rseq), reliable=False)
    if payload[0] != 0xFF:
        return ("H", payload[0]), payload[1:]
    if payload[1] != 0xFF:
        return ("M", payload[1]), payload[2:]
    return ("L", struct.unpack(">H", payload[2:4])[0]), payload[4:]


def idle(seconds):
    """Keeps the circuit alive: acknowledges and answers pings."""
    deadline = time.time() + seconds
    while time.time() < deadline:
        mid, payload = receive()
        if mid == ("H", 1):
            send(b"\x02", payload[:1], reliable=False)


def text2(text):
    data = text.encode() + b"\0"
    return struct.pack("<H", len(data)) + data


def say(text, chat_type=1):
    send(low(80), agent + session + text2(text) + bytes([chat_type]) + struct.pack("<i", 0))
    idle(0.5)


send(low(3), struct.pack("<I", code) + session + agent)
send(low(249), agent + session + struct.pack("<I", code))
position, shaken, deadline = None, False, time.time() + 15
while time.time() < deadline and not (position and shaken):
    mid, payload = receive()
    if mid == ("L", 148):
        send(low(149), agent + session + bytes(4))
        shaken = True
    elif mid == ("L", 250):
        position = struct.unpack("<3f", payload[32:44])
if not (position and shaken):
    sys.exit("handshake failed")
idle(1)



def text1(payload, at):
    size = payload[at]
    return payload[at + 1:at + 1 + size].split(b"\0")[0].decode(errors="replace"), at + 1 + size


def text2_at(payload, at):
    size = struct.unpack("<H", payload[at:at + 2])[0]
    return payload[at + 2:at + 2 + size].split(b"\0")[0].decode(errors="replace"), at + 2 + size


def reply(to, text):
    """ImprovedInstantMessage: a plain IM (dialog 0) in the conversation both ids make."""
    name = (FIRST + " " + LAST).encode() + b"\0"
    conversation = bytes(a ^ b for a, b in zip(agent, to))
    im = agent + session + bytes([0]) + to + struct.pack("<I", 0) + bytes(16) + struct.pack("<3f", *position)
    im += bytes([0, 0]) + conversation + struct.pack("<I", 0) + bytes([len(name)]) + name
    im += text2(text) + struct.pack("<H", 1) + b"\0" + struct.pack("<I", 0) + bytes([0])
    send(low(254), im)


def offer_teleport(to, text):
    """StartLure: offers `to` a teleport to where this avatar stands."""
    message = text.encode() + b"\0"
    send(low(70), agent + session + bytes([0, len(message)]) + message + bytes([1]) + to)


def objects(payload):
    """The blocks of an ObjectUpdate as (local id, full id, parent, ObjectData)."""
    at, found = 11, []
    try:
        for _ in range(payload[10]):
            local, full = struct.unpack("<I", payload[at:at + 4])[0], payload[at + 5:at + 21]
            at += 5 + 16 + 4 + 3 + 12
            size = payload[at]
            data = payload[at + 1:at + 1 + size]
            at += 1 + size
            parent = struct.unpack("<I", payload[at:at + 4])[0]
            at += 4 + 4 + 23
            for width in (2, 1, 2, 2, 1):  # TextureEntry, TextureAnim, NameValue, Data, Text
                at += width + int.from_bytes(payload[at:at + width], "little")
            at += 4
            for _ in range(3):  # MediaURL, PSBlock, ExtraParams
                at += 1 + payload[at]
            at += 16 + 16 + 4 + 1 + 4 + 1 + 12 + 12
            found.append((local, full, parent, data))
    except (IndexError, struct.error):
        pass
    return found


prim = None
if PRIM:
    # A box two metres from the bot's seat, as rezprim.py makes it.
    where = struct.pack("<3f", position[0] + 4.0, position[1] + 2.0, position[2] - 0.5)
    add = agent + session + bytes(16)
    add += bytes([9, 3]) + struct.pack("<I", 0)
    add += bytes([16, 1]) + struct.pack("<HH", 0, 0) + bytes([100, 100]) + bytes(9) + struct.pack("<HHH", 0, 0, 0)
    add += bytes([1]) + where + where + bytes(16) + bytes([0])
    add += struct.pack("<3f", 1.0, 1.0, 0.5) + bytes(12) + bytes([0])
    send(b"\xff\x01", add)
    deadline = time.time() + 10
    while time.time() < deadline and not prim:
        mid, payload = receive()
        if mid == ("H", 12) and payload[10] >= 1 and payload[36] == 9:
            prim = (struct.unpack("<I", payload[11:15])[0], uuid.UUID(bytes=payload[16:32]))
    if not prim:
        sys.exit("no ObjectUpdate for the new prim")
    # RezScript naming a library item: the simulator copies that script in.
    # OpenSim takes the prim to put it in from the folder id.
    script_name, description = b"Touch\0", b"\0"
    rez = agent + session + bytes(16) + struct.pack("<I", prim[0]) + bytes([1])
    rez += TOUCH_SCRIPT.bytes + prim[1].bytes + agent + agent + bytes(16)
    rez += struct.pack("<5I", *([0x7FFFFFFF] * 5)) + bytes([0]) + bytes(16)
    rez += bytes([10, 10]) + struct.pack("<I", 0) + bytes([0]) + struct.pack("<i", 0)
    rez += bytes([len(script_name)]) + script_name + bytes([len(description)]) + description
    rez += struct.pack("<iI", 0, 0) + bytes([0])
    send(low(304), rez)
    print("touch prim", prim[1], flush=True)

print("listening", flush=True)
started, bot_seen = time.time(), None
deadline = started + SECONDS
while time.time() < deadline:
    if OFFER_AFTER is not None and time.time() >= started + OFFER_AFTER:
        offer_teleport(BOT.bytes, "Join me, unasked")
        print("offered the bot a teleport", flush=True)
        OFFER_AFTER = None
    mid, payload = receive()
    if mid == ("H", 1):
        send(b"\x02", payload[:1], reliable=False)
    elif mid == ("H", 12):  # ObjectUpdate
        for local, full, parent, data in objects(payload):
            if full == BOT.bytes and len(data) >= 28:
                # An avatar's data starts with its collision plane.
                where = tuple(round(v, 1) for v in struct.unpack("<3f", data[16:28]))
                if (where, parent) != bot_seen:
                    bot_seen = (where, parent)
                    print("bot at %s %s %s parent %d" % (where + (parent,)), flush=True)
    elif mid == ("L", 139):  # ChatFromSimulator
        name, at = text1(payload, 0)
        chat_type = payload[at + 33]
        text, _ = text2_at(payload, at + 32 + 3 + 12)
        if chat_type not in (4, 5) and text:
            print("chat %d %s: %s" % (chat_type, name, text.replace("\n", "\\n")), flush=True)
    elif mid == ("L", 254):  # ImprovedInstantMessage
        sender, dialog, im_id = payload[:16], payload[82], uuid.UUID(bytes=payload[83:99])
        name, at = text1(payload, 103)
        text, _ = text2_at(payload, at)
        if dialog == 0:
            print("im %s %s: %s" % (name, im_id, text), flush=True)
            if sender != agent:
                reply(sender, "pong: " + text)
        elif dialog == 26:  # a request to be teleported here: grant it
            print("teleport request from %s: %s" % (name, text), flush=True)
            offer_teleport(sender, "Join me in Target")
        elif dialog == 22:
            print("teleport offer from %s: %s" % (name, text), flush=True)

if prim:
    # DeRezObject (Low 291) to the trash.
    derez = agent + session + bytes(16) + bytes([6]) + bytes(16) + uuid.uuid4().bytes + bytes([1, 0])
    send(low(291), derez + bytes([1]) + struct.pack("<I", prim[0]))
    idle(1)
send(low(252), agent + session)
time.sleep(1)

#!/usr/bin/env python3
"""Rig helper: logs in as the estate owner next to the bot and talks to it,
to exercise the bot's chat and IM logging. It says a few lines in local chat
(whisper, say, shout, and one with a tab and a line break), sends the bot an
instant message, rezzes a prim with the default new script (which says
"Script running" when it starts), takes the prim back to the trash and logs
out. Stands in for doing the same with a viewer. Start it while the bot is
seated in Target.

Usage: talk.py [BOT_AGENT_UUID]   (default: the rig's "Sit Bot" account)"""
import hashlib, re, socket, struct, sys, time, urllib.request, uuid

FIRST, LAST, PASSWORD = "Rig", "Owner", "rigowner"
LOGIN = "http://127.0.0.1:9000/"
BOT = uuid.UUID(sys.argv[1] if len(sys.argv) > 1 else "7e570000-0000-4000-8000-0000000000b0")


def member(n, v):
    return "<member><name>%s</name><value><string>%s</string></value></member>" % (n, v)


body = ("<?xml version=\"1.0\"?><methodCall><methodName>login_to_simulator</methodName><params><param><value><struct>"
        + member("first", FIRST) + member("last", LAST)
        + member("passwd", "$1$" + hashlib.md5(PASSWORD.encode()).hexdigest())
        + member("start", "uri:Target&amp;124&amp;128&amp;25") + member("channel", "rig-talker") + member("version", "0.1")
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

say("hello from the rig")
say("psst", 0)
say("HEY", 2)
say("one\ttwo\nthree \\ four")

# ImprovedInstantMessage: a plain IM (dialog 0) to the bot.
name = (FIRST + " " + LAST).encode() + b"\0"
im = agent + session + bytes([0]) + BOT.bytes + struct.pack("<I", 0) + bytes(16) + struct.pack("<3f", *position)
im += bytes([0, 0]) + bytes(16) + struct.pack("<I", 0) + bytes([len(name)]) + name
im += text2("are you there?") + struct.pack("<H", 1) + b"\0" + struct.pack("<I", 0) + bytes([0])
send(low(254), im)
idle(1)

# A prim a few metres away, as rezprim.py makes it.
where = struct.pack("<3f", position[0] + 6.0, position[1] + 3.0, position[2] - 0.5)
add = agent + session + bytes(16)
add += bytes([9, 3]) + struct.pack("<I", 0)
add += bytes([16, 1]) + struct.pack("<HH", 0, 0) + bytes([100, 100]) + bytes(9) + struct.pack("<HHH", 0, 0, 0)
add += bytes([1]) + where + where + bytes(16) + bytes([0])
add += struct.pack("<3f", 1.0, 1.0, 0.5) + bytes(12) + bytes([0])
send(b"\xff\x01", add)

prim, deadline = None, time.time() + 10
while time.time() < deadline and not prim:
    mid, payload = receive()
    if mid == ("H", 12) and payload[10] >= 1 and payload[36] == 9:
        prim = (struct.unpack("<I", payload[11:15])[0], uuid.UUID(bytes=payload[16:32]))
if not prim:
    sys.exit("no ObjectUpdate for the new prim")
local_id = struct.pack("<I", prim[0])
print("scripted prim", prim[1])

# RezScript with a null item id: the simulator adds its default new script.
# OpenSim takes the prim to put it in from the folder id.
script_name, description = b"New Script\0", b"\0"
rez = agent + session + bytes(16) + local_id + bytes([1])
rez += bytes(16) + prim[1].bytes + agent + agent + bytes(16)
rez += struct.pack("<5I", *([0x7FFFFFFF] * 5)) + bytes([0]) + bytes(16)
rez += bytes([10, 10]) + struct.pack("<I", 0) + bytes([0]) + struct.pack("<i", 0)
rez += bytes([len(script_name)]) + script_name + bytes([len(description)]) + description
rez += struct.pack("<iI", 0, 0) + bytes([0])
send(low(304), rez)
idle(8)  # compiling takes a moment; then it says "Script running"

# DeRezObject (Low 291) to the trash, then leave.
derez = agent + session + bytes(16) + bytes([6]) + bytes(16) + uuid.uuid4().bytes + bytes([1, 0])
send(low(291), derez + bytes([1]) + local_id)
idle(1)
send(low(252), agent + session)
time.sleep(1)

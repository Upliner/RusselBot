#!/usr/bin/env python3
"""Rig helper: logs in as the estate owner, rezzes one box prim in region
Target (or the region named as the first argument) next to where the avatar
lands, prints the prim's UUID and position, and logs out. Stands in for
rezzing a prim with a viewer. If it reports "already logged in", run it
again: the failed attempt clears the stale presence.

Usage: rezprim.py [REGION]"""
import hashlib, re, socket, struct, sys, time, urllib.request, uuid

FIRST, LAST, PASSWORD = "Rig", "Owner", "rigowner"
LOGIN = "http://127.0.0.1:9000/"
REGION = sys.argv[1] if len(sys.argv) > 1 else "Target"


def member(n, v):
    return "<member><name>%s</name><value><string>%s</string></value></member>" % (n, v)


body = ("<?xml version=\"1.0\"?><methodCall><methodName>login_to_simulator</methodName><params><param><value><struct>"
        + member("first", FIRST) + member("last", LAST)
        + member("passwd", "$1$" + hashlib.md5(PASSWORD.encode()).hexdigest())
        + member("start", "uri:%s&amp;124&amp;128&amp;25" % REGION) + member("channel", "rig-rezzer") + member("version", "0.1")
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
sock.settimeout(0.5)
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

# Land a few metres from where the bot stands so the avatar is on the
# ground, not on the bot's head, and put the prim at its feet level.
where = struct.pack("<3f", position[0] + 6.0, position[1], position[2] - 0.5)
add = agent + session + bytes(16)
add += bytes([9, 3]) + struct.pack("<I", 0)                  # PCode prim, Material wood, AddFlags
add += bytes([16, 1]) + struct.pack("<HH", 0, 0) + bytes([100, 100]) + bytes(9) + struct.pack("<HHH", 0, 0, 0)
add += bytes([1]) + where + where + bytes(16) + bytes([0])   # BypassRaycast, RayStart, RayEnd, RayTargetID, RayEndIsIntersection
add += struct.pack("<3f", 1.0, 1.0, 0.5) + bytes(12) + bytes([0])
send(b"\xff\x01", add)

found, deadline = None, time.time() + 10
while time.time() < deadline and not found:
    mid, payload = receive()
    if mid == ("H", 12) and payload[10] >= 1:
        full_id, pcode = payload[16:32], payload[36]
        if pcode == 9:
            found = uuid.UUID(bytes=full_id)
send(low(252), agent + session)
time.sleep(1)
if not found:
    sys.exit("no ObjectUpdate for the new prim")
print(found, "%.1f %.1f %.1f" % struct.unpack("<3f", where))

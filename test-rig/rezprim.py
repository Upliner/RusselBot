#!/usr/bin/env python3
"""Rig helper: logs in as the estate owner, rezzes one box prim in region
Target (or the region named as the first argument) next to where the avatar
lands, prints the prim's UUID and position, and logs out. Stands in for
rezzing a prim with a viewer. If it reports "already logged in", run it
again: the failed attempt clears the stale presence.

For `rslbot ctl scan` the prim can be given what a scan lists:

  name=NAME    the prim's name (quote it in the shell when it has spaces)
  desc=TEXT    its description
  touch        a script in it that says `Touched.` when it is touched
  child=NAME   a second prim named NAME, linked to the first as its child;
               its UUID is printed on a second line, after `child`
  childtouch   the script goes into the child prim instead (needs child=)
  east=N       how many metres east of the avatar the prim goes (default 6)

Usage: rezprim.py [REGION] [name=NAME] [desc=TEXT] [touch] [child=NAME] [childtouch] [east=N]"""
import hashlib, re, socket, struct, sys, time, urllib.request, uuid

FIRST, LAST, PASSWORD = "Rig", "Owner", "rigowner"
LOGIN = "http://127.0.0.1:9000/"
ARGS = sys.argv[1:]
WORDS = ("touch", "childtouch")
REGION = next((a for a in ARGS if "=" not in a and a not in WORDS), "Target")


def option(name):
    return next((a[len(name) + 1:] for a in ARGS if a.startswith(name + "=")), None)


NAME, DESC, CHILD = option("name"), option("desc"), option("child")
EAST = float(option("east") or 6.0)
TOUCH, CHILD_TOUCH = "touch" in ARGS, "childtouch" in ARGS
if CHILD_TOUCH and CHILD is None:
    sys.exit("childtouch needs child=NAME")
SCRIPT = """default
{
    touch_start(integer count)
    {
        llSay(0, "Touched.");
    }
}
"""


def member(n, v):
    return "<member><name>%s</name><value><string>%s</string></value></member>" % (n, v)


body = ("<?xml version=\"1.0\"?><methodCall><methodName>login_to_simulator</methodName><params><param><value><struct>"
        + member("first", FIRST) + member("last", LAST)
        + member("passwd", "$1$" + hashlib.md5(PASSWORD.encode()).hexdigest())
        + member("start", "uri:%s&amp;124&amp;128&amp;25" % REGION) + member("channel", "rig-rezzer") + member("version", "0.1")
        + member("platform", "Lnx") + member("mac", "0" * 32) + member("id0", "0" * 32)
        + "<member><name>agree_to_tos</name><value><boolean>1</boolean></value></member>"
        + "<member><name>options</name><value><array><data>"
        + "<value><string>inventory-root</string></value></data></array></value></member>"
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



def idle(seconds):
    """Keeps the circuit alive: acknowledges and answers pings."""
    deadline = time.time() + seconds
    while time.time() < deadline:
        mid, payload = receive()
        if mid == ("H", 1):
            send(b"\x02", payload[:1], reliable=False)


def leave(why=None):
    send(low(252), agent + session)
    time.sleep(1)
    sys.exit(why)


def text1(text):
    data = text.encode() + b"\0"
    return bytes([len(data)]) + data


taken = set()


def rez(place, size):
    """Rezzes a box at `place` and returns its local id and UUID."""
    where = struct.pack("<3f", *place)
    add = agent + session + bytes(16)
    add += bytes([9, 3]) + struct.pack("<I", 0)                  # PCode prim, Material wood, AddFlags
    add += bytes([16, 1]) + struct.pack("<HH", 0, 0) + bytes([100, 100]) + bytes(9) + struct.pack("<HHH", 0, 0, 0)
    add += bytes([1]) + where + where + bytes(16) + bytes([0])   # BypassRaycast, RayStart, RayEnd, RayTargetID, RayEndIsIntersection
    add += struct.pack("<3f", *size) + bytes(12) + bytes([0])
    send(b"\xff\x01", add)
    prim, deadline = None, time.time() + 10
    while time.time() < deadline and prim is None:
        mid, payload = receive()
        if mid == ("H", 12) and payload[10] >= 1 and payload[36] == 9:
            local = struct.unpack("<I", payload[11:15])[0]
            prim = (local, uuid.UUID(bytes=payload[16:32])) if local not in taken else None
    if prim is None:
        leave("no ObjectUpdate for the new prim")
    taken.add(prim[0])
    return prim


def upload_script():
    """Makes an inventory item of SCRIPT, as asker.py does: AssetUploadRequest
    (Low 333) with the source in the packet, then CreateInventoryItem (Low
    305) naming the same transaction. Returns the item's id."""
    root = re.search(r"<name>inventory-root</name>.*?<name>folder_id</name>\s*<value>\s*<string>([^<]*)<", reply, re.S)
    if not root:
        leave("the login reply has no inventory-root")
    folder = uuid.UUID(root.group(1))
    source, transaction = SCRIPT.encode(), uuid.uuid4()
    send(low(333), transaction.bytes + struct.pack("<b", 10) + bytes([0, 0]) + struct.pack("<H", len(source)) + source)
    uploaded, deadline = False, time.time() + 15
    while time.time() < deadline and not uploaded:
        mid, payload = receive()
        if mid == ("L", 334):  # AssetUploadComplete: UUID, Type, Success
            uploaded = bool(payload[17])
    if not uploaded:
        leave("the simulator did not take the script")
    create = struct.pack("<I", 7) + folder.bytes + transaction.bytes + struct.pack("<I", 0x7FFFFFFF)
    create += struct.pack("<bb", 10, 10) + bytes([0]) + text1("Touch script") + text1("says Touched.")
    send(low(305), agent + session + create)
    item, deadline = None, time.time() + 10
    while time.time() < deadline and item is None:
        mid, payload = receive()
        if mid == ("L", 267) and payload[33] >= 1:  # AgentID, SimApproved, TransactionID, count, ItemID
            item = uuid.UUID(bytes=payload[34:50])
    if item is None:
        leave("the simulator did not make the script item")
    return item


def put_script(prim, item):
    """RezScript (Low 304): the simulator copies the script item into the
    prim and starts it."""
    script_name, description = b"Touch script\0", b"\0"
    message = agent + session + bytes(16) + struct.pack("<I", prim[0]) + bytes([1])
    message += item.bytes + prim[1].bytes + agent + agent + bytes(16)
    message += struct.pack("<5I", *([0x7FFFFFFF] * 5)) + bytes([0]) + bytes(16)
    message += bytes([10, 10]) + struct.pack("<I", 0) + bytes([0]) + struct.pack("<i", 0)
    message += bytes([len(script_name)]) + script_name + bytes([len(description)]) + description
    message += struct.pack("<iI", 0, 0) + bytes([0])
    send(low(304), message)
    idle(5)  # compiling takes a moment


# Land a few metres from where the bot stands so the avatar is on the
# ground, not on the bot's head, and put the prim at its feet level.
place = (position[0] + EAST, position[1], position[2] - 0.5)
prim = rez(place, (1.0, 1.0, 0.5))
if NAME is not None:
    send(low(107), agent + session + bytes([1]) + struct.pack("<I", prim[0]) + text1(NAME))    # ObjectName
if DESC is not None:
    send(low(108), agent + session + bytes([1]) + struct.pack("<I", prim[0]) + text1(DESC))    # ObjectDescription
child = None
if CHILD is not None:
    child = rez((place[0], place[1], place[2] + 0.5), (0.5, 0.5, 0.5))
    send(low(107), agent + session + bytes([1]) + struct.pack("<I", child[0]) + text1(CHILD))
    idle(1)
    # ObjectLink (Low 115): the first prim named becomes the root.
    send(low(115), agent + session + bytes([2]) + struct.pack("<II", prim[0], child[0]))
    idle(1)
if TOUCH or CHILD_TOUCH:
    put_script(child if CHILD_TOUCH else prim, upload_script())
idle(1)
send(low(252), agent + session)
time.sleep(1)
print(prim[1], "%.1f %.1f %.1f" % place)
if child is not None:
    print("child", child[1])

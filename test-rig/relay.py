#!/usr/bin/env python3
"""Rig helper: gives the bot account scripted objects that say RLV commands
to their owner, which nothing else on the rig does. Logs in as `Sit Bot`
(so the bot has to be offline), uploads one script and puts it into two
boxes:

  Relay box   stays rezzed next to where the avatar lands
  Relay       is taken into the folder `RLV gear` at the top of My
              Inventory, to be attached with `rslbot ctl attach`

The script says to its owner what its owner says on channel 77, and repeats
on channel 0 what its owner says on channel 5:

  rslbot ctl say /77 @versionnew=5    the boxes tell the bot `@versionnew=5`,
                                      the bot answers on channel 5, and the
                                      boxes say `heard on 5: ...` for all

An `RLV gear` folder from an earlier run is deleted first. A `Relay box`
from an earlier run is not: delete it on the OpenSim console with
`delete object name "Relay box"`, or every command is said twice.

Usage: relay.py [REGION]   (default: Target)"""
import hashlib, re, socket, struct, sys, time, urllib.request, uuid

FIRST, LAST, PASSWORD = "Sit", "Bot", "secret"
LOGIN = "http://127.0.0.1:9000/"
REGION = sys.argv[1] if len(sys.argv) > 1 else "Target"
FOLDER = "RLV gear"
SCRIPT = """default
{
    state_entry()
    {
        llListen(77, "", llGetOwner(), "");
        llListen(5, "", llGetOwner(), "");
    }
    on_rez(integer start)
    {
        llResetScript();
    }
    changed(integer change)
    {
        if (change & CHANGED_OWNER) llResetScript();
    }
    listen(integer channel, string name, key id, string message)
    {
        if (channel == 77) llOwnerSay(message);
        else llSay(0, "heard on 5: " + message);
    }
}
"""


def member(n, v):
    return "<member><name>%s</name><value><string>%s</string></value></member>" % (n, v)


body = ("<?xml version=\"1.0\"?><methodCall><methodName>login_to_simulator</methodName><params><param><value><struct>"
        + member("first", FIRST) + member("last", LAST)
        + member("passwd", "$1$" + hashlib.md5(PASSWORD.encode()).hexdigest())
        + member("start", "uri:%s&amp;125&amp;126&amp;25" % REGION) + member("channel", "rig-relay") + member("version", "0.1")
        + member("platform", "Lnx") + member("mac", "0" * 32) + member("id0", "0" * 32)
        + "<member><name>agree_to_tos</name><value><boolean>1</boolean></value></member>"
        + "<member><name>options</name><value><array><data><value><string>inventory-root</string></value></data></array></value></member>"
        + "</struct></value></param></params></methodCall>")
reply = urllib.request.urlopen(urllib.request.Request(LOGIN, body.encode(), {"Content-Type": "text/xml"})).read().decode()


def field(name):
    found = re.findall(r"<name>%s</name>\s*<value>\s*<(?:string|int|i4)>([^<]*)<" % name, reply)
    found = [v for v in found if not v.startswith("11111111-1111")]  # skip the library owner's nested agent_id
    if not found:
        sys.exit("login failed: " + reply[:400])
    return found[0]


root = re.search(r"<name>inventory-root</name>.*?<name>folder_id</name>\s*<value>\s*<string>([^<]*)<", reply, re.S)
if not root:
    sys.exit("the login reply has no inventory-root")
root = uuid.UUID(root.group(1))
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


# The folder, as stock.py makes its own: an old one goes first.
def post(url, llsd):
    request = urllib.request.Request(url, ("<llsd>%s</llsd>" % llsd).encode(), {"Content-Type": "application/llsd+xml"})
    return urllib.request.urlopen(request, timeout=20).read().decode()


caps = post(field("seed_capability"), "<array><string>FetchInventoryDescendents2</string></array>")
fetch = re.search(r"<key>FetchInventoryDescendents2</key>\s*<string>([^<]*)<", caps).group(1)
top = post(fetch, "<map><key>folders</key><array><map><key>folder_id</key><uuid>%s</uuid>"
                  "<key>owner_id</key><uuid>%s</uuid><key>sort_order</key><integer>0</integer>"
                  "<key>fetch_folders</key><boolean>1</boolean><key>fetch_items</key><boolean>0</boolean>"
                  "</map></array></map>" % (root, uuid.UUID(bytes=agent)))
categories = re.findall(r"<map><key>category_id</key>.*?</map>", top, re.S)
category_id = lambda category: uuid.UUID(re.search(r"<key>category_id</key><uuid>([^<]*)<", category).group(1))
trash = [category_id(c) for c in categories if "<key>type_default</key><integer>14</integer>" in c]
for category in categories:
    if "<key>name</key><string>%s</string>" % FOLDER in category and trash:
        old = category_id(category)
        send(low(275), agent + session + bytes([0, 1]) + old.bytes + trash[0].bytes)
        idle(0.5)
        send(low(276), agent + session + bytes([1]) + old.bytes)
        print("removed the old %s/ (%s)" % (FOLDER, old))
idle(1)
folder = uuid.uuid4()
send(low(273), agent + session + folder.bytes + root.bytes + struct.pack("<b", -1) + text1(FOLDER))
idle(1)

# AssetUploadRequest (Low 333) with the source in the packet itself, then
# CreateInventoryItem (Low 305) naming the same transaction: the new item
# gets the uploaded asset. Asset type 10 and inventory type 10 are a script.
# UpdateCreateInventoryItem (Low 267) says which item it became.
source = SCRIPT.encode()
transaction = uuid.uuid4()
send(low(333), transaction.bytes + struct.pack("<b", 10) + bytes([0, 0]) + struct.pack("<H", len(source)) + source)
uploaded, deadline = False, time.time() + 15
while time.time() < deadline and not uploaded:
    mid, payload = receive()
    if mid == ("L", 334):  # AssetUploadComplete: UUID, Type, Success
        uploaded = bool(payload[17])
if not uploaded:
    leave("the simulator did not take the script")
create = struct.pack("<I", 7) + folder.bytes + transaction.bytes + struct.pack("<I", 0x7FFFFFFF)
create += struct.pack("<bb", 10, 10) + bytes([0]) + text1("Relay script") + text1("says what it hears")
send(low(305), agent + session + create)
script, deadline = None, time.time() + 10
while time.time() < deadline and script is None:
    mid, payload = receive()
    if mid == ("L", 267) and payload[33] >= 1:  # AgentID, SimApproved, TransactionID, count, ItemID
        script = uuid.UUID(bytes=payload[34:50])
if script is None:
    leave("the simulator did not make the script item")

taken = set()


def make_box(name, east):
    """Rezzes a small box `east` metres east of the avatar, names it and
    puts the script into it. Returns its local id and UUID."""
    where = struct.pack("<3f", position[0] + east, position[1], position[2] - 0.5)
    add = agent + session + bytes(16)
    add += bytes([9, 3]) + struct.pack("<I", 0)
    add += bytes([16, 1]) + struct.pack("<HH", 0, 0) + bytes([100, 100]) + bytes(9) + struct.pack("<HHH", 0, 0, 0)
    add += bytes([1]) + where + where + bytes(16) + bytes([0])
    add += struct.pack("<3f", 0.3, 0.3, 0.3) + bytes(12) + bytes([0])
    send(b"\xff\x01", add)
    prim, deadline = None, time.time() + 10
    while time.time() < deadline and prim is None:
        mid, payload = receive()
        if mid == ("H", 12) and payload[10] >= 1 and payload[36] == 9:
            found = struct.unpack("<I", payload[11:15])[0]
            prim = (found, uuid.UUID(bytes=payload[16:32])) if found not in taken else None
    if prim is None:
        leave("no ObjectUpdate for the new prim")
    taken.add(prim[0])
    send(low(107), agent + session + bytes([1]) + struct.pack("<I", prim[0]) + text1(name))
    idle(1)
    # RezScript (Low 304) naming the item: the simulator copies the script
    # into the prim and starts it. OpenSim takes the prim from the folder id.
    script_name, description = b"Relay script\0", b"\0"
    rez = agent + session + bytes(16) + struct.pack("<I", prim[0]) + bytes([1])
    rez += script.bytes + prim[1].bytes + agent + agent + bytes(16)
    rez += struct.pack("<5I", *([0x7FFFFFFF] * 5)) + bytes([0]) + bytes(16)
    rez += bytes([10, 10]) + struct.pack("<I", 0) + bytes([0]) + struct.pack("<i", 0)
    rez += bytes([len(script_name)]) + script_name + bytes([len(description)]) + description
    rez += struct.pack("<iI", 0, 0) + bytes([0])
    send(low(304), rez)
    idle(5)  # compiling takes a moment
    return prim


box = make_box("Relay box", 1.0)
worn = make_box("Relay", 2.0)
# DeRezObject (Low 291) with destination 4: take into the agent's
# inventory, into the folder named.
derez = agent + session + bytes(16) + bytes([4]) + folder.bytes + uuid.uuid4().bytes + bytes([1, 0])
send(low(291), derez + bytes([1]) + struct.pack("<I", worn[0]))
idle(2)

send(low(252), agent + session)
time.sleep(1)
print("relay box %s rezzed in %s at %.1f, %.1f, %.1f" % (box[1], REGION, position[0] + 1.0, position[1], position[2] - 0.5))
print("stocked %s/ (%s) with Relay" % (FOLDER, folder))

#!/usr/bin/env python3
"""Rig helper: stocks the bot account's inventory for the inventory commands.
Logs in as `Sit Bot` (so the bot has to be offline), makes the folder
`Bot gear` at the top of My Inventory and puts four items into it:

  Radio   an object: a box it rezzes at its feet and takes
  Watch   another one
  Dance   an animation: a copy of the library's `tpose2`
  Bell    a sound: OpenSim's own click sound, uploaded as a new asset

and prints what it made. Stands in for doing the same with a viewer. A
`Bot gear` folder from an earlier run is deleted first, so the items get new
UUIDs each time.

Usage: stock.py [REGION]   (default: Target)"""
import hashlib, os, re, socket, struct, sys, time, urllib.request, uuid

FIRST, LAST, PASSWORD = "Sit", "Bot", "secret"
LOGIN = "http://127.0.0.1:9000/"
REGION = sys.argv[1] if len(sys.argv) > 1 else "Target"
LIBRARY_OWNER = uuid.UUID("11111111-1111-0000-0000-000100bba000")
LIBRARY_ANIMATION = uuid.UUID("43534bdd-d00c-a9d1-874f-b83543352d54")  # library item tpose2
FOLDER = "Bot gear"
SOUND_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "opensim/bin/assets/SoundsAssetSet/OSSndClick.ogg")


def member(n, v):
    return "<member><name>%s</name><value><string>%s</string></value></member>" % (n, v)


body = ("<?xml version=\"1.0\"?><methodCall><methodName>login_to_simulator</methodName><params><param><value><struct>"
        + member("first", FIRST) + member("last", LAST)
        + member("passwd", "$1$" + hashlib.md5(PASSWORD.encode()).hexdigest())
        + member("start", "uri:%s&amp;120&amp;120&amp;25" % REGION) + member("channel", "rig-stocker") + member("version", "0.1")
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

# What is at the top of My Inventory, asked the way the bot asks: the seed
# capability gives the URL, and that gives the folders.
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
        # MoveInventoryFolder (Low 275) into the Trash, then
        # RemoveInventoryFolder (Low 276): OpenSim deletes only from there.
        send(low(275), agent + session + bytes([0, 1]) + old.bytes + trash[0].bytes)
        idle(0.5)
        send(low(276), agent + session + bytes([1]) + old.bytes)
        print("removed the old %s/ (%s)" % (FOLDER, old))
idle(1)

# CreateInventoryFolder (Low 273): an ordinary folder (type -1) at the top.
folder = uuid.uuid4()
send(low(273), agent + session + folder.bytes + root.bytes + struct.pack("<b", -1) + text1(FOLDER))
idle(1)

taken = set()


def make_object(name):
    """Rezzes a small box at the avatar's feet, names it and takes it into
    the folder."""
    # ObjectAdd, as rezprim.py does it.
    where = struct.pack("<3f", position[0] + 2.0, position[1], position[2] - 0.5)
    add = agent + session + bytes(16)
    add += bytes([9, 3]) + struct.pack("<I", 0)
    add += bytes([16, 1]) + struct.pack("<HH", 0, 0) + bytes([100, 100]) + bytes(9) + struct.pack("<HHH", 0, 0, 0)
    add += bytes([1]) + where + where + bytes(16) + bytes([0])
    add += struct.pack("<3f", 0.2, 0.2, 0.2) + bytes(12) + bytes([0])
    send(b"\xff\x01", add)
    local, deadline = None, time.time() + 10
    while time.time() < deadline and local is None:
        mid, payload = receive()
        # OwnerID is not in the update: the new prim is the first one
        # described after the request that was not taken before.
        if mid == ("H", 12) and payload[10] >= 1 and payload[36] == 9:
            found = struct.unpack("<I", payload[11:15])[0]
            local = found if found not in taken else None
    if local is None:
        send(low(252), agent + session)
        time.sleep(1)
        sys.exit("no ObjectUpdate for the new prim")
    taken.add(local)
    # ObjectName (Low 107), then DeRezObject (Low 291) with destination 4:
    # take into the agent's inventory, into the folder named.
    send(low(107), agent + session + bytes([1]) + struct.pack("<I", local) + text1(name))
    idle(1)
    derez = agent + session + bytes(16) + bytes([4]) + folder.bytes + uuid.uuid4().bytes + bytes([1, 0])
    send(low(291), derez + bytes([1]) + struct.pack("<I", local))
    idle(2)


make_object("Radio")
make_object("Watch")

# CopyInventoryItem (Low 269): the library's animation under a new name.
copy = bytes([1]) + struct.pack("<I", 1) + LIBRARY_OWNER.bytes + LIBRARY_ANIMATION.bytes + folder.bytes + text1("Dance")
send(low(269), agent + session + copy)
idle(1)

# AssetUploadRequest (Low 333) without data: the simulator asks for the
# sound with RequestXfer (Low 156) and takes it in SendXferPacket (High 18)
# pieces of 1000 bytes, the first with the total length in front and the
# last with the top bit set in its number, confirming each (High 19). Then
# CreateInventoryItem (Low 305) naming the same transaction: the new item
# gets the uploaded asset. Asset type 1 and inventory type 1 are a sound.
with open(SOUND_FILE, "rb") as file:
    sound = file.read()
transaction = uuid.uuid4()
send(low(333), transaction.bytes + struct.pack("<b", 1) + bytes([0, 0]) + struct.pack("<H", 0))
pieces = [sound[at:at + 1000] for at in range(0, len(sound), 1000)]
pieces[0] = struct.pack("<I", len(sound)) + pieces[0]
uploaded, deadline = False, time.time() + 15
while time.time() < deadline and not uploaded:
    mid, payload = receive()
    if mid == ("L", 156):  # RequestXfer: its id comes first
        xfer, number = payload[:8], 0
    elif mid == ("H", 19):  # ConfirmXferPacket: the next piece, please
        number = (struct.unpack("<I", payload[8:12])[0] & 0x7FFFFFFF) + 1
    elif mid == ("L", 334):  # AssetUploadComplete: UUID, Type, Success
        uploaded = bool(payload[17])
        continue
    else:
        continue
    if number < len(pieces):
        last = 0x80000000 if number == len(pieces) - 1 else 0
        piece = pieces[number]
        send(bytes([18]), xfer + struct.pack("<I", number | last) + struct.pack("<H", len(piece)) + piece)
if not uploaded:
    send(low(252), agent + session)
    time.sleep(1)
    sys.exit("the simulator did not take the sound")
perms = 0x7FFFFFFF
create = struct.pack("<I", 2) + folder.bytes + transaction.bytes + struct.pack("<I", perms)
create += struct.pack("<bb", 1, 1) + bytes([0]) + text1("Bell") + text1("clicks")
send(low(305), agent + session + create)
idle(2)

send(low(252), agent + session)
time.sleep(1)
print("stocked %s/ (%s) with Radio, Watch, Dance and Bell" % (FOLDER, folder))

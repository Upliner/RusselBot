#!/usr/bin/env python3
"""Rig helper: gives the rig what asks the bot's avatar for things. Logs in
as `Rig Owner` four metres from the bot, uploads one script and rezzes two
boxes with it, `Asker box` and `Asker box 2`, which stay in the region.
Each box does what anyone tells it on its channel (78 for the first box,
79 for the second), to whoever spoke:

  rslbot ctl say /78 ask 16     the box asks the bot for script permissions
                                with these bits (16: trigger animations,
                                4: take controls, 32: attach, 2: debit) and
                                says `granted: <bits>` on channel 0 when the
                                bot has answered
  rslbot ctl say /78 give       the box gives the bot a copy of its script,
                                which is an inventory offer from an object

With `give=N` the helper itself, as the avatar `Rig Owner`, gives the bot
the script from its inventory after N seconds: an inventory offer from an
avatar. It then stays for SECONDS and prints every answer it gets:

  answer <dialog> from <name>   5 accepted, 6 declined; 10 and 11 for an
                                object's offer, which go to the owner

The script item lives in the folder `Asker` at the top of My Inventory; one
from an earlier run is deleted first. Boxes from an earlier run are not:
delete them on the OpenSim console with `delete object name "Asker box"`
and `delete object name "Asker box 2"`, or every request is made twice.
`nobox` rezzes none, for a run that only gives.

Usage: asker.py [SECONDS] [give=N] [nobox]   (default: 0 seconds)"""
import hashlib, re, socket, struct, sys, time, urllib.request, uuid

FIRST, LAST, PASSWORD = "Rig", "Owner", "rigowner"
LOGIN = "http://127.0.0.1:9000/"
REGION = "Target"
ARGS = sys.argv[1:]
SECONDS = float(next((a for a in ARGS if a.replace(".", "").isdigit()), 0.0))
GIVE_AFTER = next((float(a[5:]) for a in ARGS if a.startswith("give=")), None)
BOXES = "nobox" not in ARGS
BOT = uuid.UUID("7e570000-0000-4000-8000-0000000000b0")
FOLDER = "Asker"
SCRIPT = """integer channel = 78;
default
{
    state_entry()
    {
        if (llGetObjectName() == "Asker box 2") channel = 79;
        llListen(channel, "", NULL_KEY, "");
    }
    on_rez(integer start)
    {
        llResetScript();
    }
    listen(integer heard, string name, key id, string message)
    {
        list words = llParseString2List(message, [" "], []);
        string command = llList2String(words, 0);
        if (command == "ask") llRequestPermissions(id, (integer)llList2String(words, 1));
        else if (command == "give") llGiveInventory(id, llGetScriptName());
    }
    run_time_permissions(integer granted)
    {
        llSay(0, "granted: " + (string)granted);
    }
}
"""


def member(n, v):
    return "<member><name>%s</name><value><string>%s</string></value></member>" % (n, v)


body = ("<?xml version=\"1.0\"?><methodCall><methodName>login_to_simulator</methodName><params><param><value><struct>"
        + member("first", FIRST) + member("last", LAST)
        + member("passwd", "$1$" + hashlib.md5(PASSWORD.encode()).hexdigest())
        + member("start", "uri:%s&amp;132&amp;128&amp;25" % REGION) + member("channel", "rig-asker") + member("version", "0.1")
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
create += struct.pack("<bb", 10, 10) + bytes([0]) + text1("Asker script") + text1("asks and gives")
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
    script_name, description = b"Asker script\0", b"\0"
    rez = agent + session + bytes(16) + struct.pack("<I", prim[0]) + bytes([1])
    rez += script.bytes + prim[1].bytes + agent + agent + bytes(16)
    rez += struct.pack("<5I", *([0x7FFFFFFF] * 5)) + bytes([0]) + bytes(16)
    rez += bytes([10, 10]) + struct.pack("<I", 0) + bytes([0]) + struct.pack("<i", 0)
    rez += bytes([len(script_name)]) + script_name + bytes([len(description)]) + description
    rez += struct.pack("<iI", 0, 0) + bytes([0])
    send(low(304), rez)
    idle(5)  # compiling takes a moment
    return prim



if BOXES:
    for name, east in (("Asker box", 1.0), ("Asker box 2", 2.0)):
        box = make_box(name, east)
        print("%s %s rezzed in %s at %.1f, %.1f, %.1f" % (name, box[1], REGION, position[0] + east, position[1], position[2] - 0.5),
              flush=True)


def text2(text):
    data = text.encode() + b"\0"
    return struct.pack("<H", len(data)) + data


def give(to, item):
    """ImprovedInstantMessage with dialog 4: an avatar offers `to` the
    inventory item; the bucket is the asset type (10, a script) and the
    item's id, and the message id names the transaction."""
    name = (FIRST + " " + LAST).encode() + b"\0"
    transaction = uuid.uuid4()
    im = agent + session + bytes([0]) + to.bytes + struct.pack("<I", 0) + bytes(16) + struct.pack("<3f", *position)
    im += bytes([0, 4]) + transaction.bytes + struct.pack("<I", 0) + bytes([len(name)]) + name
    im += text2("Asker script") + struct.pack("<H", 17) + bytes([10]) + item.bytes + struct.pack("<I", 0) + bytes([0])
    send(low(254), im)
    print("gave the bot Asker script (%s), transaction %s" % (item, transaction), flush=True)


started, given = time.time(), GIVE_AFTER is None
while time.time() - started < SECONDS:
    if not given and time.time() - started >= GIVE_AFTER:
        give(BOT, script)
        given = True
    mid, payload = receive()
    if mid == ("H", 1):
        send(b"\x02", payload[:1], reliable=False)
    elif mid == ("L", 254):  # ImprovedInstantMessage
        dialog = payload[82]
        size = payload[103]
        name = payload[104:104 + size].split(b"\0")[0].decode(errors="replace")
        if dialog in (5, 6, 10, 11):
            print("answer %d from %s" % (dialog, name), flush=True)

send(low(252), agent + session)
time.sleep(1)
print("script item %s in %s/ (%s)" % (script, FOLDER, folder))

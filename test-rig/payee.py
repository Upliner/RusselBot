#!/usr/bin/env python3
"""Rig helper: rezzes a box that takes money. Logs in as `Rig Owner` (or
the account --as names), as rezprim.py does, uploads a script that sets the
pay price with llSetPayPrice and has a money() event, puts it into a box
rezzed next to where the avatar lands (near the bot's place in Target),
asks for the box's pay price (RequestPayPrice) and prints the box's UUID,
position and pay price. Needs the rig's money module (`economymodule =
RigMoneyModule`) for payments to reach the script.

The box's owner gets what it is paid. When it is paid it says, in local
chat and, with --channel, on that channel too:

  paid L$<amount> by <payer name> (<payer UUID>)

Prices are numbers of L$ or the words `hide` (PAY_HIDE, -1) and `default`
(PAY_DEFAULT, -2, the viewer's own choice):

  payee.py                        the viewer's default price box and buttons
  payee.py --price hide 100 250   no price box; buttons L$100 and L$250, the other two hidden
  payee.py --price 50 hide --name "Tip jar"
                                  only the price box, with L$50 filled in

Delete a box from an earlier run on the OpenSim console with
`delete object name "Payee box"` (or the name it was given)."""
import argparse, hashlib, re, socket, struct, sys, time, urllib.request, uuid

ACCOUNTS = {
    "owner": ("Rig", "Owner", "rigowner"),
    "alt": ("Rig", "Alt", "rigalt"),
    "bot": ("Sit", "Bot", "secret"),
}
LOGIN = "http://127.0.0.1:9000/"


def price(text):
    words = {"hide": -1, "default": -2}
    if text in words:
        return words[text]
    try:
        value = int(text)
    except ValueError:
        raise argparse.ArgumentTypeError("not a number, `hide` or `default`: %r" % text)
    if value < -2:
        raise argparse.ArgumentTypeError("prices are L$0 or more, -1 (hide) or -2 (default)")
    return value


parser = argparse.ArgumentParser(
    description=__doc__.split("\n\n")[0],
    epilog="\n\n".join(__doc__.split("\n\n")[1:]), formatter_class=argparse.RawDescriptionHelpFormatter)
parser.add_argument("buttons", type=price, nargs="*", metavar="BUTTON",
                    help="up to four quick-pay buttons; those not given are hidden (with none given: `default` each)")
parser.add_argument("--price", type=price, default=-2, metavar="PRICE",
                    help="the price box: L$, `hide` or `default` (default: default)")
parser.add_argument("--name", default="Payee box", help='the box\'s name (default: "Payee box")')
parser.add_argument("--channel", type=int, default=0,
                    help="a channel to say the payments on as well as local chat (default: none)")
parser.add_argument("--as", dest="who", choices=sorted(ACCOUNTS), default="owner",
                    help="the account that rezzes and owns the box (default: owner)")
parser.add_argument("--region", default="Target", help="the region (default: Target)")
parser.add_argument("--east", type=float, default=6.0, metavar="N",
                    help="metres east of the avatar's landing place, 124,128 (default: 6)")
parser.add_argument("--north", type=float, default=3.0, metavar="N",
                    help="metres north of it (default: 3, so not on rezprim.py's box)")
args = parser.parse_args()
if len(args.buttons) > 4:
    parser.error("at most four buttons")
BUTTONS = args.buttons + [-1 if args.buttons else -2] * (4 - len(args.buttons))
FIRST, LAST, PASSWORD = ACCOUNTS[args.who]

SCRIPT = """default
{
    state_entry()
    {
        llSetPayPrice(%d, [%d, %d, %d, %d]);
    }
    on_rez(integer start)
    {
        llResetScript();
    }
    money(key id, integer amount)
    {
        string line = "paid L$" + (string)amount + " by " + llKey2Name(id) + " (" + (string)id + ")";
        llSay(0, line);
        if (%d != 0)
            llSay(%d, line);
    }
}
""" % (args.price, *BUTTONS, args.channel, args.channel)


def member(n, v):
    return "<member><name>%s</name><value><string>%s</string></value></member>" % (n, v)


body = ("<?xml version=\"1.0\"?><methodCall><methodName>login_to_simulator</methodName><params><param><value><struct>"
        + member("first", FIRST) + member("last", LAST)
        + member("passwd", "$1$" + hashlib.md5(PASSWORD.encode()).hexdigest())
        + member("start", "uri:%s&amp;124&amp;128&amp;25" % args.region) + member("channel", "rig-payee") + member("version", "0.1")
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


def rez(place, size):
    """Rezzes a box at `place` and returns its local id and UUID, as rezprim.py does."""
    where = struct.pack("<3f", *place)
    add = agent + session + bytes(16)
    add += bytes([9, 3]) + struct.pack("<I", 0)                  # PCode prim, Material wood, AddFlags
    add += bytes([16, 1]) + struct.pack("<HH", 0, 0) + bytes([100, 100]) + bytes(9) + struct.pack("<HHH", 0, 0, 0)
    add += bytes([1]) + where + where + bytes(16) + bytes([0])   # BypassRaycast, RayStart, RayEnd, RayTargetID, RayEndIsIntersection
    add += struct.pack("<3f", *size) + bytes(12) + bytes([0])
    send(b"\xff\x01", add)
    deadline = time.time() + 10
    while time.time() < deadline:
        mid, payload = receive()
        if mid == ("H", 12) and payload[10] >= 1 and payload[36] == 9:
            return struct.unpack("<I", payload[11:15])[0], uuid.UUID(bytes=payload[16:32])
    leave("no ObjectUpdate for the new prim")


def upload_script():
    """AssetUploadRequest (Low 333) with the source in the packet, then
    CreateInventoryItem (Low 305), as rezprim.py does. Returns the item's id."""
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
    create += struct.pack("<bb", 10, 10) + bytes([0]) + text1("Payee script") + text1("says what it is paid")
    send(low(305), agent + session + create)
    deadline = time.time() + 10
    while time.time() < deadline:
        mid, payload = receive()
        if mid == ("L", 267) and payload[33] >= 1:  # AgentID, SimApproved, TransactionID, count, ItemID
            return uuid.UUID(bytes=payload[34:50])
    leave("the simulator did not make the script item")


def put_script(prim, item):
    """RezScript (Low 304): the simulator copies the script item into the prim and starts it."""
    script_name, description = b"Payee script\0", b"\0"
    message = agent + session + bytes(16) + struct.pack("<I", prim[0]) + bytes([1])
    message += item.bytes + prim[1].bytes + agent + agent + bytes(16)
    message += struct.pack("<5I", *([0x7FFFFFFF] * 5)) + bytes([0]) + bytes(16)
    message += bytes([10, 10]) + struct.pack("<I", 0) + bytes([0]) + struct.pack("<i", 0)
    message += bytes([len(script_name)]) + script_name + bytes([len(description)]) + description
    message += struct.pack("<iI", 0, 0) + bytes([0])
    send(low(304), message)
    idle(5)  # compiling takes a moment


def name(text):
    return "%d (%s)" % (text, {-1: "hide", -2: "default"}[text]) if text < 0 else "L$%d" % text


place = (position[0] + args.east, position[1] + args.north, position[2] - 0.5)
prim = rez(place, (0.5, 0.5, 0.5))
send(low(107), agent + session + bytes([1]) + struct.pack("<I", prim[0]) + text1(args.name))  # ObjectName
put_script(prim, upload_script())

# RequestPayPrice (Low 161); the answer is PayPriceReply (Low 162).
send(low(161), prim[1].bytes)
paid, deadline = None, time.time() + 5
while time.time() < deadline and paid is None:
    mid, payload = receive()
    if mid == ("L", 162) and payload[:16] == prim[1].bytes:
        paid = struct.unpack("<i", payload[16:20])[0], struct.unpack("<%di" % payload[20], payload[21:21 + 4 * payload[20]])
    elif mid == ("H", 1):
        send(b"\x02", payload[:1], reliable=False)
send(low(252), agent + session)
time.sleep(1)
print(prim[1], "%.1f %.1f %.1f" % place, args.name)
if paid is None:
    sys.exit("no PayPriceReply: is the money module on? (the simulator sends none for prices of all zeros)")
print("pay price:", name(paid[0]), "buttons:", ", ".join(name(b) for b in paid[1]))

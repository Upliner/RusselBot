#!/usr/bin/env python3
"""Rig helper: logs in as an avatar other than the bot (`Rig Owner` unless
--as says otherwise), as talk.py does, and pays: an avatar (by default the
bot, `Sit Bot`) with a MoneyTransferRequest of type 5001 (gift), or with
--object a prim with type 5008 (pay object). Needs the rig's money module
(`economymodule = RigMoneyModule`). Stands in for the Pay dialog of a
viewer.

It asks for its balance first (MoneyBalanceRequest), for an object also
for the pay price (RequestPayPrice), then pays, and prints every
MoneyBalanceReply and PayPriceReply it gets, and what is said in local chat
(the payee box of payee.py says what it was paid), until --wait seconds
after the payment.

Accounts (--as, and names for TO):

  owner  Rig Owner  7e570000-0000-4000-8000-0000000000aa  (password rigowner)
  alt    Rig Alt    7e570000-0000-4000-8000-0000000000a1  (password rigalt)
  bot    Sit Bot    7e570000-0000-4000-8000-0000000000b0  (password secret)

Examples:

  pay.py 100                      Rig Owner gives Sit Bot L$100
  pay.py 25 "Rig Alt"             Rig Owner gives Rig Alt L$25
  pay.py --as alt --object UUID 100
                                  Rig Alt pays the prim UUID L$100
  pay.py --balance                only asks for Rig Owner's balance
  pay.py --balance --wait 30      and stays 30 s, printing what it is told
                                  about payments to Rig Owner
"""
import argparse, hashlib, re, socket, struct, sys, time, urllib.request, uuid

ACCOUNTS = {
    "owner": ("Rig", "Owner", "rigowner", "7e570000-0000-4000-8000-0000000000aa"),
    "alt": ("Rig", "Alt", "rigalt", "7e570000-0000-4000-8000-0000000000a1"),
    "bot": ("Sit", "Bot", "secret", "7e570000-0000-4000-8000-0000000000b0"),
}
LOGIN = "http://127.0.0.1:9000/"
NAMES = {}  # message number -> name, for --hex
TYPES = {5001: "gift", 5008: "pay object", 5009: "object pays"}


def avatar(text):
    """A UUID, an account key of ACCOUNTS or the full name of one."""
    for key, (first, last, _, uid) in ACCOUNTS.items():
        if text.lower() in (key, (first + " " + last).lower()):
            return uuid.UUID(uid)
    try:
        return uuid.UUID(text)
    except ValueError:
        raise argparse.ArgumentTypeError("not a UUID nor one of the rig's accounts: %r" % text)


parser = argparse.ArgumentParser(
    description=__doc__.split("\n\n")[0],
    epilog="\n\n".join(__doc__.split("\n\n")[1:]), formatter_class=argparse.RawDescriptionHelpFormatter)
parser.add_argument("amount", type=int, nargs="?", help="L$ to pay (leave out with --balance)")
parser.add_argument("to", type=avatar, nargs="?", default=avatar("bot"),
                    help='the avatar to pay: a UUID or "Sit Bot", "Rig Owner", "Rig Alt" (default: Sit Bot)')
parser.add_argument("--as", dest="who", choices=sorted(ACCOUNTS), default="owner",
                    help="the account to log in as (default: owner)")
parser.add_argument("--object", type=uuid.UUID, metavar="UUID",
                    help="pay this prim instead of an avatar (type 5008); its owner gets the money")
parser.add_argument("--description", default="", metavar="TEXT",
                    help="the transfer's Description (a viewer sends the object's name for --object)")
parser.add_argument("--balance", action="store_true", help="only ask for the balance; pay nothing")
parser.add_argument("--region", default="Target", help="the region to log in to (default: Target)")
parser.add_argument("--wait", type=float, default=3.0, metavar="SECONDS",
                    help="how long to listen after paying, or with --balance after the balance, for payments"
                         " others make (default: 3)")
parser.add_argument("--hex", action="store_true",
                    help="also print the money messages sent and received as hex, from the message number on, zerocoding undone")
args = parser.parse_args()
if (args.amount is None) != args.balance:
    parser.error("give an AMOUNT, or --balance alone")

FIRST, LAST, PASSWORD, _ = ACCOUNTS[args.who]


def member(n, v):
    return "<member><name>%s</name><value><string>%s</string></value></member>" % (n, v)


body = ("<?xml version=\"1.0\"?><methodCall><methodName>login_to_simulator</methodName><params><param><value><struct>"
        + member("first", FIRST) + member("last", LAST)
        + member("passwd", "$1$" + hashlib.md5(PASSWORD.encode()).hexdigest())
        + member("start", "uri:%s&amp;122&amp;124&amp;25" % args.region) + member("channel", "rig-payer") + member("version", "0.1")
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


def show_hex(direction, name, data):
    if args.hex:
        print("%s %s %s" % (direction, name, data.hex()))


def send(mid, payload, reliable=True, name=None):
    global seq
    seq += 1
    if name:
        show_hex(">", name, mid + payload)
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
    """Returns the message number, its body after the number, and the whole
    body from the number on."""
    try:
        data, _ = sock.recvfrom(65536)
    except socket.timeout:
        return None, b"", b""
    flags, rseq, extra = data[0], struct.unpack(">I", data[1:5])[0], data[5]
    payload = data[6 + extra:]
    if flags & 0x10:
        payload = payload[: -1 - 4 * payload[-1]]
    if flags & 0x80:
        payload = zdecode(payload)
    if flags & 0x40:
        send(b"\xff\xff\xff\xfb", bytes([1]) + struct.pack("<I", rseq), reliable=False)
    if payload[0] != 0xFF:
        return ("H", payload[0]), payload[1:], payload
    if payload[1] != 0xFF:
        return ("M", payload[1]), payload[2:], payload
    return ("L", struct.unpack(">H", payload[2:4])[0]), payload[4:], payload


def text1(text):
    data = text.encode() + b"\0" if text else b""
    return bytes([len(data)]) + data


def var(data, at, size):
    """A Variable field at `at` with a length of `size` bytes: its text and the offset after it."""
    n = int.from_bytes(data[at:at + size], "little")
    return data[at + size:at + size + n].rstrip(b"\0").decode(errors="replace"), at + size + n


def name_of(uid):
    for first, last, _, known in ACCOUNTS.values():
        if uid == uuid.UUID(known):
            return "%s %s" % (first, last)
    return str(uid)


def handle(mid, payload, whole):
    """Prints the messages of interest; answers pings."""
    if mid == ("H", 1):
        send(b"\x02", payload[:1], reliable=False)
    elif mid == ("L", 314):  # MoneyBalanceReply
        show_hex("<", "MoneyBalanceReply", whole)
        transaction = uuid.UUID(bytes=payload[16:32])
        success, balance = payload[32], struct.unpack("<i", payload[33:37])[0]
        description, at = var(payload, 45, 1)
        line = "MoneyBalanceReply: success=%s balance=L$%d transaction=%s description=%r" % (
            bool(success), balance, transaction, description)
        if len(payload) > at:
            kind = struct.unpack("<i", payload[at:at + 4])[0]
            source, source_group = uuid.UUID(bytes=payload[at + 4:at + 20]), payload[at + 20]
            dest, dest_group = uuid.UUID(bytes=payload[at + 21:at + 37]), payload[at + 37]
            amount = struct.unpack("<i", payload[at + 38:at + 42])[0]
            item, _ = var(payload, at + 42, 1)
            if kind:
                line += "\n  TransactionInfo: type=%d (%s) source=%s%s dest=%s%s amount=L$%d item=%r" % (
                    kind, TYPES.get(kind, "?"), name_of(source), " (group)" if source_group else "",
                    name_of(dest), " (group)" if dest_group else "", amount, item)
            else:
                line += "\n  TransactionInfo: none (type 0)"
        print(line, flush=True)
    elif mid == ("L", 162):  # PayPriceReply
        show_hex("<", "PayPriceReply", whole)
        default = struct.unpack("<i", payload[16:20])[0]
        buttons = struct.unpack("<%di" % payload[20], payload[21:21 + 4 * payload[20]])
        print("PayPriceReply: object=%s default=%d buttons=%s" % (
            uuid.UUID(bytes=payload[:16]), default, list(buttons)), flush=True)
    elif mid == ("L", 139):  # ChatFromSimulator
        name, at = var(payload, 0, 1)
        message, _ = var(payload, at + 32 + 3 + 12, 2)
        if payload[at + 33] in (0, 1, 2):  # whisper, say, shout
            print("chat: %s: %s" % (name, message), flush=True)


def idle(seconds):
    deadline = time.time() + seconds
    while time.time() < deadline:
        handle(*receive())


send(low(3), struct.pack("<I", code) + session + agent)
send(low(249), agent + session + struct.pack("<I", code))
moved, shaken, deadline = False, False, time.time() + 15
while time.time() < deadline and not (moved and shaken):
    mid, payload, whole = receive()
    if mid == ("L", 148):
        send(low(149), agent + session + bytes(4))
        shaken = True
    elif mid == ("L", 250):
        moved = True
    else:
        handle(mid, payload, whole)
if not (moved and shaken):
    sys.exit("handshake failed")
print("logged in as %s %s (%s)" % (FIRST, LAST, uuid.UUID(bytes=agent)), flush=True)
idle(1)

# MoneyBalanceRequest (Low 313): AgentData, TransactionID.
send(low(313), agent + session + uuid.uuid4().bytes, name="MoneyBalanceRequest")
idle(1.5)

if not args.balance:
    if args.object:
        # RequestPayPrice (Low 161), as the viewer asks before its Pay dialog.
        send(low(161), args.object.bytes, name="RequestPayPrice")
        idle(1.5)
        kind, dest = 5008, args.object
    else:
        kind, dest = 5001, args.to
    # MoneyTransferRequest (Low 311): AgentData; SourceID, DestID, Flags,
    # Amount, AggregatePermNextOwner, AggregatePermInventory,
    # TransactionType, Description.
    money = agent + dest.bytes + bytes([0]) + struct.pack("<i", args.amount) + bytes([0, 0])
    money += struct.pack("<i", kind) + text1(args.description)
    send(low(311), agent + session + money, name="MoneyTransferRequest")
    print("paid L$%d to %s (type %d)" % (args.amount, name_of(dest), kind), flush=True)
idle(args.wait)

send(low(252), agent + session)
time.sleep(1)

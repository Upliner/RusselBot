#!/usr/bin/env python3
"""Rig helper for groups: logs in as `Rig Owner` (or another rig account),
as listen.py does, and does one thing with OpenSim's Groups V2. Stands in
for a viewer's group windows.

Usage: group.py [as=owner|alt|bot] [dump] [wait=SECONDS] COMMAND ARGS...

  create NAME [fee=N] [open]   create a group, listed in search, with a fee
                               of L$N and open enrollment when asked; prints
                               its UUID. The creator is its owner.
  invite GROUP [WHO]           invite WHO (default: the bot, `Sit Bot`) into
                               GROUP, role Everyone; WHO is owner, alt, bot
                               or a UUID
  say GROUP TEXT...            say a line in GROUP's chat, starting the
                               session first as a viewer does
  notice GROUP SUBJECT TEXT [item]
                               send GROUP a notice; with `item`, a copy of
                               the library animation `tpose2` named `Agenda`
                               is attached
  eject GROUP [WHO]            eject WHO (default: the bot) from GROUP
  join GROUP                   join GROUP (an open one, by UUID); prints the
                               JoinGroupReply and the MoneyBalanceReply of a
                               fee
  leave GROUP                  leave GROUP; prints the LeaveGroupReply
  profile GROUP                ask for GROUP's profile and print it
  list                         print the account's groups
  listen [SECONDS]             print the group chat, notices and every
                               instant message the account gets, with its
                               dialog (default: 60 seconds)

GROUP is a UUID or the name of one of the account's groups. Every command
listens for `wait=` seconds (default 3) before it logs out, and prints what
arrives meanwhile. `dump` also prints, for the group messages received, the
body as hex (after the message number, zerocoding undone), and the LLSD of
the event-queue events about groups and chat sessions, for captures.

Output lines:
  groups: <uuid> <name> (powers <hex>, notices <0|1>)   active: <uuid> <name> title <title>
  im <dialog> from <name> <id>: <text> [id <uuid>] [bucket <hex>]
  group chat <group>: <name>: <text>       notice <group>: <name>: <subject> | <text> [attachment]
  invitation into <group> from <name>: fee L$<n> id <uuid>
  session start reply <group>: success <0|1> <error>
  JoinGroupReply <group> success <0|1>     MoneyBalanceReply ...
  LeaveGroupReply <group> success <0|1>"""
import hashlib, queue, re, socket, struct, sys, threading, time, urllib.request, uuid
from xml.sax.saxutils import escape

ACCOUNTS = {
    "owner": ("Rig", "Owner", "rigowner", "7e570000-0000-4000-8000-0000000000aa"),
    "alt": ("Rig", "Alt", "rigalt", "7e570000-0000-4000-8000-0000000000a1"),
    "bot": ("Sit", "Bot", "secret", "7e570000-0000-4000-8000-0000000000b0"),
}
LOGIN = "http://127.0.0.1:9000/"
LIBRARY_OWNER = uuid.UUID("11111111-1111-0000-0000-000100bba000")
LIBRARY_ANIMATION = uuid.UUID("43534bdd-d00c-a9d1-874f-b83543352d54")  # library item tpose2

ARGS = sys.argv[1:]
WHO = next((a[3:] for a in ARGS if a.startswith("as=")), "owner")
DUMP = "dump" in ARGS
WAIT = float(next((a[5:] for a in ARGS if a.startswith("wait=")), "3"))
WORDS = [a for a in ARGS if not a.startswith(("as=", "wait=")) and a != "dump"]
if not WORDS or WHO not in ACCOUNTS:
    sys.exit(__doc__)
COMMAND, REST = WORDS[0], WORDS[1:]
FIRST, LAST, PASSWORD, _ = ACCOUNTS[WHO]


def person(text):
    if text in ACCOUNTS:
        return uuid.UUID(ACCOUNTS[text][3])
    return uuid.UUID(text)


def name_of(uid):
    for first, last, _, known in ACCOUNTS.values():
        if uid == uuid.UUID(known):
            return "%s %s" % (first, last)
    return str(uid)


# ------------------------------------------------------------------ login

def member(n, v):
    return "<member><name>%s</name><value><string>%s</string></value></member>" % (n, v)


body = ("<?xml version=\"1.0\"?><methodCall><methodName>login_to_simulator</methodName><params><param><value><struct>"
        + member("first", FIRST) + member("last", LAST)
        + member("passwd", "$1$" + hashlib.md5(PASSWORD.encode()).hexdigest())
        + member("start", "uri:Target&amp;120&amp;132&amp;25") + member("channel", "rig-groups") + member("version", "0.1")
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


agent, session = uuid.UUID(field("agent_id")).bytes, uuid.UUID(field("session_id")).bytes
code, peer = int(field("circuit_code")), (field("sim_ip"), int(field("sim_port")))
root = uuid.UUID(re.search(r"<name>inventory-root</name>.*?<name>folder_id</name>\s*<value>\s*<string>([^<]*)<",
                           reply, re.S).group(1))
sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.settimeout(0.2)
seq = 0
lock = threading.Lock()


def send(mid, payload, reliable=True):
    global seq
    with lock:
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


def text1(text):
    data = text.encode() + b"\0" if text else b""
    return bytes([len(data)]) + data


def text2(text):
    data = text.encode() + b"\0" if text else b""
    return struct.pack("<H", len(data)) + data


def var(data, at, size):
    """A Variable field at `at`: its text and the offset after it."""
    n = int.from_bytes(data[at:at + size], "little")
    return data[at + size:at + size + n].rstrip(b"\0").decode(errors="replace"), at + size + n


def instant_message(to, dialog, ident, text, bucket=b"\0", name=None):
    """ImprovedInstantMessage (Low 254) from the account."""
    block = agent + session + bytes([0]) + to.bytes + bytes(4) + bytes(16) + bytes(12)
    block += bytes([0, dialog]) + ident.bytes + bytes(4)
    block += text1(name or "%s %s" % (FIRST, LAST)) + text2(text) + struct.pack("<H", len(bucket)) + bucket
    block += bytes(4) + bytes([0])
    send(low(254), block)


# ----------------------------------------------------------- event queue

events = queue.Queue()
groups = {}  # UUID -> name
started = {}  # session -> (success, error)


def post(url, llsd):
    request = urllib.request.Request(url, ("<llsd>%s</llsd>" % llsd).encode(), {"Content-Type": "application/llsd+xml"})
    return urllib.request.urlopen(request, timeout=40).read().decode()


caps = post(field("seed_capability"), "<array><string>EventQueueGet</string></array>")
queue_url = re.search(r"<key>EventQueueGet</key>\s*<string>([^<]*)<", caps).group(1)
running = True


def poll():
    ack = "<undef />"
    while running:
        try:
            got = post(queue_url, "<map><key>ack</key>%s<key>done</key><boolean>0</boolean></map>" % ack)
        except Exception:
            time.sleep(0.5)
            continue
        found = re.search(r"<key>id</key><integer>(\d+)</integer>", got)
        if found:
            ack = "<integer>%s</integer>" % found.group(1)
        # Each event runs to the next one; the last to the end of the reply.
        for chunk in got.split("<map><key>message</key><string>")[1:]:
            name, _, body = chunk.partition("</string>")
            events.put((name, body))


threading.Thread(target=poll, daemon=True).start()


def uuid_after(key, text):
    found = re.search(r"<key>%s</key><uuid>([^<]*)</uuid>" % key, text)
    return uuid.UUID(found.group(1)) if found else None


def string_after(key, text):
    found = re.search(r"<key>%s</key>(?:<string>([^<]*)</string>|<string />)" % key, text)
    return (found.group(1) or "") if found else ""


def handle_event(name, body):
    if DUMP and name in ("AgentGroupDataUpdate", "AgentDropGroup", "ChatterBoxInvitation", "ChatterBoxSessionStartReply"):
        print("event %s: %s" % (name, body), flush=True)
    if name == "AgentGroupDataUpdate":
        groups.clear()
        for block in re.findall(r"<map><key>GroupID</key>.*?</map>", body, re.S):
            gid = uuid_after("GroupID", block)
            groups[gid] = string_after("GroupName", block)
            notices = re.search(r"<key>AcceptNotices</key><boolean>(\d)</boolean>", block)
            print("groups: %s %s (notices %s)" % (gid, groups[gid], notices.group(1) if notices else "?"), flush=True)
        if not groups:
            print("groups: none", flush=True)
    elif name == "ChatterBoxInvitation":
        session_id = uuid_after("session_id", body)
        message = string_after("message", body.split("<key>message_params</key>", 1)[-1])
        sender = string_after("from_name", body)
        print("group chat %s: %s: %s (first line, as an event)" % (groups.get(session_id, session_id), sender, message), flush=True)
    elif name == "ChatterBoxSessionStartReply":
        session_id = uuid_after("session_id", body)
        success = re.search(r"<key>success</key><boolean>(\d)</boolean>", body)
        ok = success is not None and success.group(1) == "1"
        started[session_id] = (ok, string_after("error", body))
        print("session start reply %s: success %d %s" % (groups.get(session_id, session_id), ok, started[session_id][1]), flush=True)
    elif name == "AgentDropGroup":
        print("dropped from %s" % uuid_after("GroupID", body), flush=True)


# --------------------------------------------------------------- simulator

NAMES = {387: "AgentDataUpdate", 389: "AgentGroupDataUpdate", 390: "AgentDropGroup", 352: "GroupProfileReply",
         344: "JoinGroupReply", 348: "LeaveGroupReply", 254: "ImprovedInstantMessage", 340: "CreateGroupReply", 314: "MoneyBalanceReply"}
created = []


def handle(mid, payload):
    if mid is None:
        return
    if mid == ("H", 1):
        send(b"\x02", payload[:1], reliable=False)
        return
    if mid[0] != "L" or mid[1] not in NAMES:
        return
    number = mid[1]
    if DUMP and number != 254:
        print("udp %s: %s" % (NAMES[number], payload.hex()), flush=True)
    if number == 387:
        at = 16
        _, at = var(payload, at, 1)
        _, at = var(payload, at, 1)
        title, at = var(payload, at, 1)
        active = uuid.UUID(bytes=payload[at:at + 16])
        name, _ = var(payload, at + 24, 1)
        print("active: %s %s title %r" % (active, name, title), flush=True)
    elif number == 389:
        count, at = payload[16], 17
        for _ in range(count):
            gid = uuid.UUID(bytes=payload[at:at + 16])
            powers = struct.unpack("<Q", payload[at + 16:at + 24])[0]
            notices = payload[at + 24]
            name, at = var(payload, at + 45, 1)
            groups[gid] = name
            print("groups: %s %s (powers %x, notices %d)" % (gid, name, powers, notices), flush=True)
    elif number == 390:
        print("dropped from %s" % uuid.UUID(bytes=payload[16:32]), flush=True)
    elif number == 352:
        gid = uuid.UUID(bytes=payload[16:32])
        name, at = var(payload, 32, 1)
        charter, at = var(payload, at, 2)
        listed = payload[at]
        title, at = var(payload, at + 1, 1)
        founder = uuid.UUID(bytes=payload[at + 24:at + 40])
        fee, open_ = struct.unpack("<i", payload[at + 40:at + 44])[0], payload[at + 44]
        members, roles = struct.unpack("<ii", payload[at + 49:at + 57])
        print("profile %s: name %r founder %s fee L$%d open %d members %d roles %d listed %d title %r charter %r" % (
            gid, name, name_of(founder), fee, open_, members, roles, listed, title, charter), flush=True)
    elif number in (344, 348):
        print("%s %s success %d" % (NAMES[number], uuid.UUID(bytes=payload[16:32]), payload[32]), flush=True)
    elif number == 340:
        gid = uuid.UUID(bytes=payload[16:32])
        message, _ = var(payload, 33, 1)
        created.append((gid, payload[32], message))
    elif number == 314:
        success, balance = payload[32], struct.unpack("<i", payload[33:37])[0]
        description, at = var(payload, 45, 1)
        line = "MoneyBalanceReply: success=%s balance=L$%d description=%r" % (bool(success), balance, description)
        if len(payload) > at:
            kind = struct.unpack("<i", payload[at:at + 4])[0]
            source = uuid.UUID(bytes=payload[at + 4:at + 20])
            amount = struct.unpack("<i", payload[at + 38:at + 42])[0]
            item, _ = var(payload, at + 42, 1)
            line += "\n  TransactionInfo: type=%d source=%s amount=L$%d item=%r" % (kind, name_of(source), amount, item)
        print(line, flush=True)
    elif number == 254:
        from_agent = uuid.UUID(bytes=payload[0:16])
        dialog = payload[33 + 16 + 4 + 16 + 12 + 1]
        at = 33 + 16 + 4 + 16 + 12 + 2
        ident = uuid.UUID(bytes=payload[at:at + 16])
        name, at = var(payload, at + 20, 1)
        text, at = var(payload, at, 2)
        size = struct.unpack("<H", payload[at:at + 2])[0]
        bucket = payload[at + 2:at + 2 + size]
        if DUMP and dialog in (3, 15, 17, 32):
            print("udp ImprovedInstantMessage (dialog %d): %s" % (dialog, payload.hex()), flush=True)
        if dialog == 17:
            print("group chat %s: %s: %s" % (groups.get(ident, ident), name, text), flush=True)
        elif dialog == 32:
            subject, _, rest = text.partition("|")
            group = uuid.UUID(bytes=bucket[2:18]) if len(bucket) >= 18 else None
            item = bucket[18:].rstrip(b"\0").decode(errors="replace") if bucket[:1] == b"\1" else ""
            print("notice %s: %s: %s | %s%s" % (groups.get(group, group), name, subject, rest,
                                                " [attachment %s]" % item if item else ""), flush=True)
        elif dialog == 3:
            fee = struct.unpack(">i", bucket[:4])[0] if len(bucket) >= 4 else None
            print("invitation into %s from %s: fee L$%s id %s" % (from_agent, name, fee, ident), flush=True)
        else:
            print("im %d from %s %s: %s [id %s] [bucket %s]" % (dialog, name, from_agent, text, ident, bucket.hex()), flush=True)


def idle(seconds):
    deadline = time.time() + seconds
    while time.time() < deadline:
        handle(*receive())
        while not events.empty():
            handle_event(*events.get())


send(low(3), struct.pack("<I", code) + session + agent)
send(low(249), agent + session + struct.pack("<I", code))
moved, shaken, deadline = False, False, time.time() + 15
while time.time() < deadline and not (moved and shaken):
    mid, payload = receive()
    if mid == ("L", 148):
        send(low(149), agent + session + bytes(4))
        shaken = True
    elif mid == ("L", 250):
        moved = True
    else:
        handle(mid, payload)
if not (moved and shaken):
    sys.exit("handshake failed")
print("logged in as %s %s (%s)" % (FIRST, LAST, uuid.UUID(bytes=agent)), flush=True)
# AgentDataUpdateRequest (Low 386), as a viewer asks after login.
send(low(386), agent + session)
idle(2)


def group(text):
    try:
        return uuid.UUID(text)
    except ValueError:
        named = [gid for gid, name in groups.items() if name.lower() == text.lower()]
        if len(named) != 1:
            finish("%s is not one of the account's groups: %s" % (text, groups))
        return named[0]


def finish(error=None):
    global running
    idle(WAIT)
    running = False
    send(low(252), agent + session)
    time.sleep(1)
    if error:
        sys.exit(error)
    sys.exit(0)


if COMMAND == "create" and REST:
    fee = int(next((a[4:] for a in REST if a.startswith("fee=")), "0"))
    is_open = "open" in REST
    name = " ".join(a for a in REST if not a.startswith("fee=") and a != "open")
    # CreateGroupRequest (Low 339): Name, Charter, ShowInList, InsigniaID,
    # MembershipFee, OpenEnrollment, AllowPublish, MaturePublish.
    data = text1(name) + text2("A group of the rig.") + bytes([1]) + bytes(16)
    data += struct.pack("<i", fee) + bytes([1 if is_open else 0, 1, 0])
    send(low(339), agent + session + data)
    deadline = time.time() + 10
    while time.time() < deadline and not created:
        idle(0.2)
    if not created:
        finish("no CreateGroupReply")
    gid, success, message = created[0]
    if not success:
        finish("not created: %s" % message)
    print("created %s %s (fee L$%d, %s)" % (gid, name, fee, "open" if is_open else "closed"), flush=True)
elif COMMAND == "invite" and REST:
    gid, invitee = group(REST[0]), person(REST[1] if len(REST) > 1 else "bot")
    # InviteGroupRequest (Low 349): GroupID, then InviteeID and RoleID.
    send(low(349), agent + session + gid.bytes + bytes([1]) + invitee.bytes + bytes(16))
    print("invited %s into %s" % (name_of(invitee), groups.get(gid, gid)), flush=True)
elif COMMAND == "say" and len(REST) >= 2:
    gid, text = group(REST[0]), " ".join(REST[1:])
    instant_message(gid, 15, gid, "")
    deadline = time.time() + 5
    while time.time() < deadline and gid not in started:
        idle(0.2)
    instant_message(gid, 17, gid, text)
    print("said in %s: %s" % (groups.get(gid, gid), text), flush=True)
elif COMMAND == "notice" and len(REST) >= 3:
    gid, subject, text = group(REST[0]), REST[1], REST[2]
    bucket = b"\0"
    if len(REST) > 3 and REST[3] == "item":
        # CopyInventoryItem (Low 269): the library animation into the top
        # folder as `Agenda`; its UUID comes back with the new item.
        send(low(269), agent + session + bytes([1]) + struct.pack("<I", 7) + LIBRARY_OWNER.bytes
             + LIBRARY_ANIMATION.bytes + root.bytes + text1("Agenda"))
        item, deadline = None, time.time() + 10
        while time.time() < deadline and item is None:
            mid, payload = receive()
            # UpdateCreateInventoryItem (Low 267): AgentID, SimApproved,
            # TransactionID, then the items; BulkUpdateInventory (Low 281):
            # AgentID, TransactionID, no folders, then the items.
            if (mid == ("L", 267) and payload[33] >= 1) or (mid == ("L", 281) and payload[32] == 0 and payload[33] >= 1):
                item = uuid.UUID(bytes=payload[34:50])
            else:
                if DUMP and mid and mid[0] == "L":
                    print("after the copy: Low %d" % mid[1], flush=True)
                handle(mid, payload)
        if item is None:
            finish("the copy of the library item did not come back")
        llsd = ("<llsd><map><key>item_id</key><uuid>%s</uuid><key>owner_id</key><uuid>%s</uuid></map></llsd>"
                % (item, uuid.UUID(bytes=agent)))
        # OpenSim drops the first 15 characters of the bucket.
        bucket = b"\x01" + b" " * 14 + llsd.encode() + b"\0"
    instant_message(gid, 32, uuid.UUID(int=0), "%s|%s" % (subject, text), bucket)
    print("sent %s the notice %r" % (groups.get(gid, gid), subject), flush=True)
elif COMMAND == "eject" and REST:
    gid, ejectee = group(REST[0]), person(REST[1] if len(REST) > 1 else "bot")
    # EjectGroupMemberRequest (Low 345): GroupID, then EjecteeID.
    send(low(345), agent + session + gid.bytes + bytes([1]) + ejectee.bytes)
    print("ejected %s from %s" % (name_of(ejectee), groups.get(gid, gid)), flush=True)
elif COMMAND == "join" and REST:
    gid = uuid.UUID(REST[0])
    send(low(313), agent + session + bytes(16))  # the balance before
    idle(1)
    send(low(343), agent + session + gid.bytes)
    print("asked to join %s" % gid, flush=True)
elif COMMAND == "leave" and REST:
    gid = group(REST[0])
    # LeaveGroupRequest (Low 347): GroupID.
    send(low(347), agent + session + gid.bytes)
    print("asked to leave %s" % groups.get(gid, gid), flush=True)
elif COMMAND == "profile" and REST:
    gid = group(REST[0])
    send(low(351), agent + session + gid.bytes)
elif COMMAND == "list":
    pass
elif COMMAND == "listen":
    WAIT = float(REST[0]) if REST else 60.0
else:
    finish(__doc__)
finish()

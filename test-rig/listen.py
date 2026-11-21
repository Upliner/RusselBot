#!/usr/bin/env python3
"""Rig helper: logs in as the estate owner next to the bot and listens, to
exercise the bot's control socket. For SECONDS it prints every line of local
chat it hears and every instant message it gets, and answers each instant
message from the bot with "pong: <text>", but never an automatic answer
(dialog 20), which it prints with its dialog. It also prints where the bot's
avatar is and what it is attached to whenever that changes, answers the
bot's request for a teleport with an offer, prints the offers the bot
sends, and says when the bot's avatar starts or stops sitting on the ground,
what is attached to it, which animations it plays and which sounds it
triggers. Stands in for watching with a viewer. Start it while the bot is
seated in Target, then use `rslbot ctl`.

Usage: listen.py [SECONDS] [prim] [offer=N] [im=T1,T2,...] [at=X,Y,Z]   (default: 30 seconds)

  prim      rez a box near the bot with the library script DefaultWithTouch,
            which says "Touched." when clicked, and print its UUID; the box
            goes to the trash at the end
  offer=N   after N seconds, offer the bot a teleport it did not ask for
  at=X,Y,Z  log in at that place in Target instead of 124, 128, 25
  im=T,...  T seconds after listening starts, send the bot the instant
            message "hello <T>", for each T given

Output lines:
  chat <type> <name>: <text>          im <name> <id>: <text>
  autoresponse <name> <id>: <text>    (dialog 20)      sent im: <text>
  bot at <x> <y> <z> parent <local id>   (parent 0: not sitting on an object)
  teleport request from <name>: <text>   teleport offer from <name>: <text>
  touch prim <uuid>
  bot ground sit: yes|no                 (from the animations it plays)
  bot attachment <item uuid> on point <N>     bot attachment <item uuid> gone
  bot plays <animation asset uuid>       bot stops <animation asset uuid>
  bot sound <sound asset uuid> at <x> <y> <z> gain <g>
Chat on channels other than 0 reaches scripts only, so it never shows here."""
import hashlib, re, socket, struct, sys, time, urllib.request, uuid

FIRST, LAST, PASSWORD = "Rig", "Owner", "rigowner"
LOGIN = "http://127.0.0.1:9000/"
ARGS = sys.argv[1:]
SECONDS = float(next((a for a in ARGS if a.replace(".", "").isdigit()), 30.0))
PRIM = "prim" in ARGS
OFFER_AFTER = next((float(a[6:]) for a in ARGS if a.startswith("offer=")), None)
AT = next((a[3:].split(",") for a in ARGS if a.startswith("at=")), ["124", "128", "25"])
IM_AT = sorted(float(t) for a in ARGS if a.startswith("im=") for t in a[3:].split(",") if t)
BOT = uuid.UUID("7e570000-0000-4000-8000-0000000000b0")
# SIT_GROUND and SIT_GROUND_CONSTRAINED, from OpenSim's avataranimations.xml.
GROUND_SITS = {uuid.UUID("1c7600d6-661f-b87b-efe2-d7421eb93c86").bytes,
               uuid.UUID("1a2bd58e-87ff-0df8-0b4c-53e047b0bb6e").bytes}
TOUCH_SCRIPT = uuid.UUID("9267e689-4f37-4ce5-b2c2-455ee2e1ab12")  # library item DefaultWithTouch


def member(n, v):
    return "<member><name>%s</name><value><string>%s</string></value></member>" % (n, v)


body = ("<?xml version=\"1.0\"?><methodCall><methodName>login_to_simulator</methodName><params><param><value><struct>"
        + member("first", FIRST) + member("last", LAST)
        + member("passwd", "$1$" + hashlib.md5(PASSWORD.encode()).hexdigest())
        + member("start", "uri:Target&amp;%s&amp;%s&amp;%s" % tuple(AT)) + member("channel", "rig-listener") + member("version", "0.1")
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


# What the simulator says about objects and animations before the listening
# starts: the bot's avatar and its attachments are described right at login.
backlog = []


def idle(seconds):
    """Keeps the circuit alive: acknowledges and answers pings."""
    deadline = time.time() + seconds
    while time.time() < deadline:
        mid, payload = receive()
        if mid == ("H", 1):
            send(b"\x02", payload[:1], reliable=False)
        elif mid in (("H", 12), ("H", 20)):
            backlog.append((mid, payload))


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
    elif mid in (("H", 12), ("H", 20)):
        backlog.append((mid, payload))
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
    """The blocks of an ObjectUpdate as (local id, full id, parent,
    ObjectData, State, NameValue)."""
    at, found = 11, []
    try:
        for _ in range(payload[10]):
            local, full = struct.unpack("<I", payload[at:at + 4])[0], payload[at + 5:at + 21]
            state = payload[at + 4]
            at += 5 + 16 + 4 + 3 + 12
            size = payload[at]
            data = payload[at + 1:at + 1 + size]
            at += 1 + size
            parent = struct.unpack("<I", payload[at:at + 4])[0]
            at += 4 + 4 + 23
            fields = []
            for width in (2, 1, 2, 2, 1):  # TextureEntry, TextureAnim, NameValue, Data, Text
                size = int.from_bytes(payload[at:at + width], "little")
                fields.append(payload[at + width:at + width + size])
                at += width + size
            at += 4
            for _ in range(3):  # MediaURL, PSBlock, ExtraParams
                at += 1 + payload[at]
            at += 16 + 16 + 4 + 1 + 4 + 1 + 12 + 12
            found.append((local, full, parent, data, state, fields[2]))
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
started, bot_seen, bot_on_ground = time.time(), None, None
# The bot avatar's local id and this avatar's own; the attachments
# described, by local id, as (item, point, parent); those reported as the
# bot's; its animations. OpenSim describes the bot's avatar to this client
# only when something about it changes, so until then an attachment on any
# avatar but this one is taken to be the bot's: the rig has just the two.
bot_local, own_local, worn, reported, bot_plays = None, None, {}, set(), set()
deadline = started + SECONDS
while time.time() < deadline:
    if OFFER_AFTER is not None and time.time() >= started + OFFER_AFTER:
        offer_teleport(BOT.bytes, "Join me, unasked")
        print("offered the bot a teleport", flush=True)
        OFFER_AFTER = None
    while IM_AT and time.time() >= started + IM_AT[0]:
        text = "hello %g" % IM_AT.pop(0)
        reply(BOT.bytes, text)
        print("sent im:", text, flush=True)
    mid, payload = backlog.pop(0) if backlog else receive()
    if mid == ("H", 1):
        send(b"\x02", payload[:1], reliable=False)
    elif mid == ("H", 12):  # ObjectUpdate
        for local, full, parent, data, state, names in objects(payload):
            if full == agent:
                own_local = local
            if full == BOT.bytes and len(data) >= 28:
                bot_local = local
                # An avatar's data starts with its collision plane.
                where = tuple(round(v, 1) for v in struct.unpack("<3f", data[16:28]))
                if (where, parent) != bot_seen:
                    bot_seen = (where, parent)
                    print("bot at %s %s %s parent %d" % (where + (parent,)), flush=True)
            # An attachment's root prim names its inventory item; its point
            # is the State with the nibbles swapped.
            item = re.search(rb"AttachItemID STRING RW SV ([0-9a-f-]{36})", names)
            if item:
                worn[local] = (item.group(1).decode(), (state >> 4) | ((state & 0x0F) << 4), parent)
        for local, (item, point, parent) in worn.items():
            on_bot = parent == bot_local if bot_local else parent not in (0, own_local)
            if on_bot and local not in reported:
                reported.add(local)
                print("bot attachment %s on point %d" % (item, point), flush=True)
    elif mid == ("H", 16):  # KillObject
        for i in range(payload[0]):
            local = struct.unpack("<I", payload[1 + 4 * i:5 + 4 * i])[0]
            item = worn.pop(local, None)
            if local in reported:
                reported.discard(local)
                print("bot attachment %s gone" % item[0], flush=True)
    elif mid == ("H", 20) and payload[:16] == BOT.bytes:  # AvatarAnimation
        playing = {payload[17 + 20 * i:33 + 20 * i] for i in range(payload[16])}
        on_ground = bool(playing & GROUND_SITS)
        if on_ground != bot_on_ground:
            bot_on_ground = on_ground
            print("bot ground sit: %s" % ("yes" if on_ground else "no"), flush=True)
        for animation in sorted(playing - bot_plays):
            print("bot plays %s" % uuid.UUID(bytes=animation), flush=True)
        for animation in sorted(bot_plays - playing):
            print("bot stops %s" % uuid.UUID(bytes=animation), flush=True)
        bot_plays = playing
    elif mid == ("H", 29) and payload[16:32] == BOT.bytes:  # SoundTrigger: SoundID, OwnerID, ...
        where = tuple(round(v, 1) for v in struct.unpack("<3f", payload[72:84]))
        gain = struct.unpack("<f", payload[84:88])[0]
        print("bot sound %s at %s %s %s gain %.1f" % ((uuid.UUID(bytes=payload[:16]),) + where + (gain,)), flush=True)
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
        elif dialog == 20:  # an automatic answer: printed, never answered
            print("autoresponse %s %s: %s" % (name, im_id, text), flush=True)
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

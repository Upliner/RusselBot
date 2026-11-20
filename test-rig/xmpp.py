#!/usr/bin/env python3
"""A tiny XMPP client for the rig's Prosody (no TLS, SASL PLAIN).

    test-rig/xmpp.py owner [--resource R] [--wait SECONDS] [ACTION...]

Logs in as ACCOUNT@rig.test (the password is the account name, as
`setup.sh` registers it), sends available presence, carries out the
actions in order and prints every stanza it receives, one per line, until
WAIT seconds (default 5) have passed after the last action. Actions:

    msg:TO:BODY       a chat message to TO (BODY may hold colons)
    sub:TO            a subscription request to TO
    unsub:TO          an unsubscription from TO
    probe:TO          a presence probe to TO
    iq:TO:XML         an iq get to TO with XML as its child
    raw:XML           any XML as it is
    sleep:SECONDS     wait before the next action

Without actions it reads lines `TO BODY` from standard input and sends
each as a chat message, printing what arrives meanwhile.
"""

import base64
import itertools
import re
import socket
import sys
import threading
import time
from xml.sax.saxutils import escape, quoteattr

DOMAIN = "rig.test"
ids = itertools.count(1)


def recv_until(sock, pattern, buf=b""):
    while not re.search(pattern, buf):
        chunk = sock.recv(65536)
        if not chunk:
            raise SystemExit(f"connection closed; got {buf!r}")
        buf += chunk
    return buf


def login(account, resource):
    sock = socket.create_connection(("127.0.0.1", 5222))
    header = (
        f"<?xml version='1.0'?><stream:stream to='{DOMAIN}' version='1.0' "
        "xmlns='jabber:client' xmlns:stream='http://etherx.jabber.org/streams'>"
    )
    sock.sendall(header.encode())
    recv_until(sock, rb"</stream:features>")
    token = base64.b64encode(f"\0{account}\0{account}".encode()).decode()
    sock.sendall(
        f"<auth xmlns='urn:ietf:params:xml:ns:xmpp-sasl' mechanism='PLAIN'>{token}</auth>".encode()
    )
    answer = recv_until(sock, rb"<success|<failure")
    if b"<failure" in answer:
        raise SystemExit(f"login refused: {answer!r}")
    sock.sendall(header.encode())
    recv_until(sock, rb"</stream:features>")
    sock.sendall(
        "<iq type='set' id='bind'><bind xmlns='urn:ietf:params:xml:ns:xmpp-bind'>"
        f"<resource>{escape(resource)}</resource></bind></iq>".encode()
    )
    bound = recv_until(sock, rb"</iq>")
    jid = re.search(rb"<jid>([^<]*)</jid>", bound).group(1).decode()
    print(f"# logged in as {jid}", flush=True)
    sock.sendall(b"<iq type='get' id='roster'><query xmlns='jabber:iq:roster'/></iq><presence/>")
    return sock


def reader(sock):
    buf = b""
    while True:
        try:
            chunk = sock.recv(65536)
        except OSError:
            return
        if not chunk:
            print("# stream closed", flush=True)
            return
        buf += chunk
        # Prosody sends one stanza per write; print what came, a stanza a line.
        text = buf.decode(errors="replace")
        buf = b""
        for part in re.split(r"(?<=>)(?=<(?:message|presence|iq)\b)", text):
            if part.strip():
                print(part.strip(), flush=True)


def act(sock, action):
    kind, _, rest = action.partition(":")
    if kind == "msg":
        to, _, body = rest.partition(":")
        stanza = f"<message type='chat' to={quoteattr(to)} id='m{next(ids)}'><body>{escape(body)}</body></message>"
    elif kind in ("sub", "unsub"):
        word = "subscribe" if kind == "sub" else "unsubscribe"
        stanza = f"<presence type='{word}' to={quoteattr(rest)}/>"
    elif kind == "probe":
        stanza = f"<presence type='probe' to={quoteattr(rest)}/>"
    elif kind == "iq":
        to, _, child = rest.partition(":")
        stanza = f"<iq type='get' to={quoteattr(to)} id='q{next(ids)}'>{child}</iq>"
    elif kind == "raw":
        stanza = rest
    elif kind == "sleep":
        time.sleep(float(rest))
        return
    else:
        raise SystemExit(f"unknown action {action!r}")
    print(f"> {stanza}", flush=True)
    sock.sendall(stanza.encode())


def main():
    args = sys.argv[1:]
    if not args or args[0].startswith("-"):
        raise SystemExit(__doc__)
    account, args = args[0], args[1:]
    resource, wait = "rig", 5.0
    while args and args[0].startswith("--"):
        option, value, args = args[0], args[1], args[2:]
        if option == "--resource":
            resource = value
        elif option == "--wait":
            wait = float(value)
        else:
            raise SystemExit(f"unknown option {option}")
    sock = login(account, resource)
    threading.Thread(target=reader, args=(sock,), daemon=True).start()
    if args:
        for action in args:
            act(sock, action)
        time.sleep(wait)
    else:
        for line in sys.stdin:
            to, _, body = line.rstrip("\n").partition(" ")
            if to:
                act(sock, f"msg:{to}:{body}")
        time.sleep(wait)
    sock.sendall(b"</stream:stream>")
    sock.close()


if __name__ == "__main__":
    main()

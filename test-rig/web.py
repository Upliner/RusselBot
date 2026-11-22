#!/usr/bin/env python3
"""A tiny client for the bot's web interface, without a browser.

    test-rig/web.py [--at HOST:PORT] [--password P] [--wait SECONDS] [ACTION...]

Talks to the web interface at HOST:PORT (default 127.0.0.1:8080) as its
page does. With --password it logs in first and keeps the session for the
actions. Carries out the actions in order and prints what each is
answered; then, if any action opened the event stream, prints events
until WAIT seconds (default 5) have passed. Actions:

    cmd:NAME:ARG:...   a command as the page sends one: its name and its
                       arguments, separated by colons (`\\:` is a colon
                       inside an argument), e.g. cmd:tp:128:128:25:Home
    own:NAME:ARG:...   the same, marked as the page's own refresh
    line:TEXT          a console line as it is typed, e.g. line:scan 20 -n bench
    events             open the event stream and print what arrives
    events:LAST        the same, going on after the event with the id LAST
    session            ask whether a login is needed: open, in or login
    logout             end the session
    raw:METHOD:PATH    a bare request without the page's marker, e.g.
                       raw:POST:/api/command, to see it refused
    host:NAME          send the following requests under this Host header
    sleep:SECONDS      wait before the next action

An answer is printed as `ok`, `ok: TEXT` or `error: TEXT`, with the
listing's own tabs and line ends; a request the web interface refuses is
printed as its status and text. Events are printed as `TYPE [ID] FIELDS`,
the fields separated by ` | `.
"""

import socket
import sys
import threading
import time

address = ("127.0.0.1", 8080)
cookie = None
host = None
printing = threading.Lock()


def say(text):
    with printing:
        print(text, flush=True)


def escape(field):
    return field.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r")


def unescape(field):
    out, chars = [], iter(field)
    for char in chars:
        if char != "\\":
            out.append(char)
            continue
        following = next(chars, "")
        out.append({"\\": "\\", "t": "\t", "n": "\n", "r": "\r"}.get(following, "\\" + following))
    return "".join(out)


def split_action(text):
    """The parts of an action between colons; `\\:` is a colon in a part."""
    parts, part, chars = [], [], iter(text)
    for char in chars:
        if char == "\\":
            following = next(chars, "")
            part.append(following if following == ":" else "\\" + following)
        elif char == ":":
            parts.append("".join(part))
            part = []
        else:
            part.append(char)
    parts.append("".join(part))
    return parts


def head_of(method, path, body, marker=True, own=False):
    lines = [f"{method} {path} HTTP/1.1", f"Host: {host or '%s:%d' % address}"]
    if marker:
        lines.append("X-Rslbot: 1")
    if own:
        lines.append("X-Rslbot-Own: 1")
    if cookie:
        lines.append(f"Cookie: {cookie}")
    if method == "POST":
        lines.append(f"Content-Length: {len(body)}")
    return ("\r\n".join(lines) + "\r\n\r\n").encode() + body


def request(method, path, body=b"", marker=True, own=False):
    """One request; the status, the headers and the body of the answer."""
    with socket.create_connection(address, timeout=90) as sock:
        sock.sendall(head_of(method, path, body, marker, own))
        answer = b""
        while chunk := sock.recv(65536):
            answer += chunk
    head, _, text = answer.partition(b"\r\n\r\n")
    lines = head.decode(errors="replace").split("\r\n")
    status = int(lines[0].split(" ")[1]) if lines and len(lines[0].split(" ")) > 1 else 0
    headers = dict(line.split(": ", 1) for line in lines[1:] if ": " in line)
    return status, headers, text.decode(errors="replace")


def show_answer(status, text):
    if status != 200:
        say(f"{status}: {text.strip()}")
        return
    line = text[:-1] if text.endswith("\n") else text
    word, _, rest = line.partition("\t")
    rest = unescape(rest)
    say(word if not rest else f"{word}: {rest}")


def login(password):
    global cookie
    status, headers, text = request("POST", "/api/login", password.encode())
    if status != 204:
        raise SystemExit(f"login refused: {status} {text.strip()}")
    cookie = headers.get("Set-Cookie", "").split(";")[0]


def events(last):
    """Prints the events of a stream until the stream ends."""
    path = "/api/events" + (f"?last={last}" if last else "")
    sock = socket.create_connection(address, timeout=None)
    sock.sendall(head_of("GET", path, b"", marker=False))
    reader = sock.makefile("rb")
    status = reader.readline().decode(errors="replace").strip()
    while reader.readline().strip():
        pass
    if " 200 " not in status + " ":
        say(f"events: {status} {reader.read().decode(errors='replace').strip()}")
        return
    kind, ident, data = "", "", ""
    for raw in reader:
        line = raw.decode(errors="replace").rstrip("\n")
        if line.startswith("event: "):
            kind = line[7:]
        elif line.startswith("id: "):
            ident = line[4:]
        elif line.startswith("data: "):
            data = line[6:]
        elif line == "" and kind:
            fields = " | ".join(unescape(field).replace("\n", "\\n") for field in data.split("\t"))
            say(f"{kind} [{ident}] {fields}" if ident else f"{kind} {fields}")
            kind, ident, data = "", "", ""
    say("events: the stream ended")


def main():
    global address, host, cookie
    args = sys.argv[1:]
    wait, password = 5.0, None
    while args and args[0].startswith("--"):
        option = args.pop(0)
        if option == "--at":
            name, _, port = args.pop(0).rpartition(":")
            address = (name.strip("[]"), int(port))
        elif option == "--password":
            password = args.pop(0)
        elif option == "--wait":
            wait = float(args.pop(0))
        else:
            raise SystemExit(__doc__)
    if not args:
        raise SystemExit(__doc__)
    if password is not None:
        login(password)
    streaming = False
    for action in args:
        parts = split_action(action)
        name = parts[0]
        if name in ("cmd", "own"):
            line = "\t".join(escape(part) for part in parts[1:]).encode()
            status, _, text = request("POST", "/api/command", line, own=name == "own")
            show_answer(status, text)
        elif name == "line":
            status, _, text = request("POST", "/api/console", action[5:].encode())
            show_answer(status, text)
        elif name == "events":
            last = parts[1] if len(parts) > 1 else None
            threading.Thread(target=events, args=(last,), daemon=True).start()
            streaming = True
            time.sleep(0.3)
        elif name == "session":
            status, _, text = request("GET", "/api/session")
            say(f"{status}: {text.strip()}")
        elif name == "logout":
            status, _, text = request("POST", "/api/logout")
            say(f"{status}: logged out" if status == 204 else f"{status}: {text.strip()}")
            cookie = None
        elif name == "raw":
            method, path = parts[1], ":".join(parts[2:])
            status, _, text = request(method, path, b"say\thello" if method == "POST" else b"", marker=False)
            say(f"{status}: {text.strip()[:200]}")
        elif name == "host":
            host = ":".join(parts[1:])
        elif name == "sleep":
            time.sleep(float(parts[1]))
        else:
            raise SystemExit(f"unknown action: {action}\n\n{__doc__}")
    if streaming:
        time.sleep(wait)


if __name__ == "__main__":
    main()

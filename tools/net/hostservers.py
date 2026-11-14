#!/usr/bin/env python3
"""Small servers on the PC side of the network examples.

    python3 tools/net/hostservers.py           # echo on 7777 (TCP) and 7778 (UDP),
                                               # stdio sink on 7779, HTTP on 7780

Then write this PC's address into sd:/citronnade/net.txt on the console's SD
card (for example `192.168.1.20`). tools/emu/check.py starts the same servers
on 127.0.0.1 for the headless checks (`host_servers` in an example's
emulator metadata).
"""

import http.server
import socket
import socketserver
import sys
import threading

ECHO_TCP, ECHO_UDP, SINK_TCP, HTTP = 7777, 7778, 7779, 7780

DEFAULT_FILES = {
    "/hello.txt": ("text/plain", b"Hello from the PC!\n"),
    "/status.json": (
        "application/json",
        b'{"name": "citronnade", "examples": 40, "temperature": 21.5, "lemons": ["eureka", "lisbon", "meyer"]}',
    ),
}


def _serve(target, *args):
    t = threading.Thread(target=target, args=args, daemon=True)
    t.start()
    return t


def tcp_echo(host="0.0.0.0", port=ECHO_TCP):
    """Echoes every TCP connection's data back."""
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host, port))
    srv.listen(8)

    def client(conn):
        with conn:
            while data := conn.recv(4096):
                conn.sendall(data)

    def loop():
        while True:
            conn, _ = srv.accept()
            _serve(client, conn)

    _serve(loop)
    return srv


def udp_echo(host="0.0.0.0", port=ECHO_UDP):
    """Answers every datagram with b"echo:" + the datagram."""
    srv = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host, port))

    def loop():
        while True:
            data, addr = srv.recvfrom(2048)
            srv.sendto(b"echo:" + data, addr)

    _serve(loop)
    return srv


def tcp_sink(host="0.0.0.0", port=SINK_TCP, echo_to=None):
    """Collects (and optionally prints) everything sent to it; returns
    (socket, list of received byte strings)."""
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host, port))
    srv.listen(8)
    received = []

    def client(conn):
        with conn:
            while data := conn.recv(4096):
                received.append(data)
                if echo_to:
                    echo_to.write(data.decode("utf-8", "replace"))
                    echo_to.flush()

    def loop():
        while True:
            conn, _ = srv.accept()
            _serve(client, conn)

    _serve(loop)
    return srv, received


def http_server(host="0.0.0.0", port=HTTP, files=None):
    """Serves `files` ({path: (content type, bytes)})."""
    files = files or DEFAULT_FILES

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            entry = files.get(self.path.split("?")[0])
            if entry is None:
                self.send_error(404)
                return
            kind, body = entry
            self.send_response(200)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            n = int(self.headers.get("Content-Length", "0"))
            body = self.rfile.read(n)
            reply = b'{"received": %d}' % len(body)
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(reply)))
            self.end_headers()
            self.wfile.write(reply)

        def log_message(self, *args):
            pass

    class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
        allow_reuse_address = True
        daemon_threads = True

    srv = Server((host, port), Handler)
    _serve(srv.serve_forever)
    return srv


if __name__ == "__main__":
    tcp_echo()
    udp_echo()
    tcp_sink(echo_to=sys.stdout)
    http_server()
    print(f"echo: TCP {ECHO_TCP}, UDP {ECHO_UDP}; stdio sink: TCP {SINK_TCP}; HTTP: {HTTP}. Ctrl-C stops.")
    try:
        threading.Event().wait()
    except KeyboardInterrupt:
        pass

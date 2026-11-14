#!/usr/bin/env python3
"""HTTP proxy double for scripts/test-linux-gui.sh.

Refuses every request and appends its first line to a log, so the tests
reach no server and the log names each one they tried to reach. Requests are
answered one at a time, in the order they arrive.
"""

import socketserver
import sys


class Refuse(socketserver.StreamRequestHandler):
    timeout = 5

    def handle(self):
        try:
            request = self.rfile.readline(4096).decode("latin-1").strip()
        except OSError:
            request = ""
        with open(self.server.log, "a", encoding="utf-8") as log:
            log.write(f"{request or '(no request line)'}\n")
        try:
            self.wfile.write(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
        except OSError:
            pass


def main():
    log, port = sys.argv[1:]
    with socketserver.TCPServer(("127.0.0.1", 0), Refuse) as server:
        server.log = log
        with open(port, "w", encoding="utf-8") as ready:
            ready.write(f"{server.server_address[1]}\n")
        server.serve_forever()


if __name__ == "__main__":
    main()

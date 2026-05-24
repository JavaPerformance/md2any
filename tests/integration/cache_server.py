"""Tiny HTTP server with controllable misbehaviour for stress-testing the
md2any remote-image cache.

Run directly:
    python3 tests/integration/cache_server.py [PORT]

Or let `cache_stress.sh` lifecycle-manage it.

Routes:
  /good.png       valid 32x16 RGBA PNG
  /flaky.png      503 first 2 requests, 200 after — exercises retry loop
  /empty.png      200 OK with zero body — exercises empty-body rejection
  /garbage.png    200 OK with HTML body — exercises sniff-before-cache
  /huge.png       30 MB of zeros — exercises 20 MB cap
  /count          plain-text request counter (used by harness assertions)
  /reset          zero the flaky counter
"""
import http.server
import struct
import sys
import zlib


def make_png(w=32, h=16):
    """Build a valid RGBA PNG by hand so we don't need PIL/imagemagick."""
    sig = b"\x89PNG\r\n\x1a\n"

    def chunk(tag, data):
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)  # 8-bit RGBA
    raw = b"".join(b"\x00" + b"\xff\x00\x88\xff" * w for _ in range(h))
    idat = zlib.compress(raw)
    return sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b"")


PNG = make_png()
HUGE = b"\x00" * (30 * 1024 * 1024)


class State:
    flaky_count = 0
    request_count = 0


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        # Suppress access logs — they clutter the harness output.
        pass

    def do_GET(self):
        State.request_count += 1
        path = self.path.split("?", 1)[0]

        if path == "/good.png":
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(PNG)))
            self.end_headers()
            self.wfile.write(PNG)

        elif path == "/flaky.png":
            State.flaky_count += 1
            if State.flaky_count < 3:
                self.send_response(503)
                self.send_header("Retry-After", "1")
                self.send_header("Content-Type", "text/plain")
                self.end_headers()
                self.wfile.write(b"come back soon")
            else:
                self.send_response(200)
                self.send_header("Content-Type", "image/png")
                self.end_headers()
                self.wfile.write(PNG)

        elif path == "/empty.png":
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", "0")
            self.end_headers()

        elif path == "/garbage.png":
            body = b"<html><body>not an image</body></html>"
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        elif path == "/huge.png":
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(HUGE)))
            self.end_headers()
            try:
                self.wfile.write(HUGE)
            except (BrokenPipeError, ConnectionResetError):
                pass

        elif path == "/count":
            body = str(State.request_count).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        elif path == "/reset":
            State.flaky_count = 0
            self.send_response(200)
            self.send_header("Content-Length", "2")
            self.end_headers()
            self.wfile.write(b"ok")

        else:
            self.send_response(404)
            self.end_headers()


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18421
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"listening on 127.0.0.1:{port}", flush=True)
    server.serve_forever()

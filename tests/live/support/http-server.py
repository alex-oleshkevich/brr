import http.server
import sys

port, directory = int(sys.argv[1]), sys.argv[2]
download = "--download" in sys.argv[3:]


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=directory, **kwargs)

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        if download and self.path.split("?")[0].endswith(".bin"):
            self.send_header("Content-Disposition", 'attachment; filename="attachment.bin"')
        super().end_headers()

    def log_message(self, *args):
        pass


http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()

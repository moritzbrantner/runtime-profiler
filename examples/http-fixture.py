"""Disposable loopback fixture for examples/http-workload.json, not a production API."""

import http.server
import os
from pathlib import Path


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"fixture-ready")

    def log_message(self, *_args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
Path(os.environ["RUNTIME_PROFILER_PORT_FILE"]).write_text(str(server.server_port))
server.serve_forever()

"""An isolated real PengChain server and temporary trusted HTTPS fixture."""
from contextlib import contextmanager
import http.client
import http.server
import json
import os
from pathlib import Path
import socket
import ssl
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[3]


@contextmanager
def fixture():
    with tempfile.TemporaryDirectory() as directory:
        temp = Path(directory)
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            backend_port = listener.getsockname()[1]
        env = dict(os.environ, PENGCHAIN_DB=str(temp / 'ledger.sqlite'), HOST='127.0.0.1', PORT=str(backend_port))
        server = subprocess.Popen(['node', 'dist/index.js'], cwd=ROOT / 'server', env=env, stdout=subprocess.DEVNULL)
        proxy = None
        try:
            for attempt in range(100):
                connection = http.client.HTTPConnection('127.0.0.1', backend_port, timeout=1)
                try:
                    connection.request('GET', '/health')
                    if connection.getresponse().status == 200:
                        break
                except OSError:
                    time.sleep(.05)
                finally:
                    connection.close()
            else:
                raise RuntimeError('Server did not start')
            connection = http.client.HTTPConnection('127.0.0.1', backend_port)
            connection.request('POST', '/blockchain/api/wallet/create', json.dumps({'password': 'android-test-password'}), {'Content-Type': 'application/json'})
            response = connection.getresponse()
            assert response.status == 200, response.status
            wallet = json.loads(response.read())['walletId']
            connection.close()
            subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', str(temp / 'key.pem'), '-out', str(temp / 'cert.pem'), '-days', '1', '-subj', '/CN=localhost', '-addext', 'subjectAltName=DNS:localhost,IP:10.0.2.2'], check=True, capture_output=True)

            class Proxy(http.server.BaseHTTPRequestHandler):
                def forward(self):
                    upstream = http.client.HTTPConnection('127.0.0.1', backend_port, timeout=20)
                    try:
                        upstream.request(self.command, self.path, self.rfile.read(int(self.headers.get('Content-Length', 0))), dict(self.headers))
                        result = upstream.getresponse()
                        body = result.read()
                        self.send_response(result.status)
                        for name, value in result.getheaders():
                            if name.lower() not in ('connection', 'transfer-encoding', 'content-length'):
                                self.send_header(name, value)
                        self.send_header('Content-Length', str(len(body)))
                        self.end_headers()
                        self.wfile.write(body)
                    finally:
                        upstream.close()

                do_GET = forward
                do_POST = forward

                def log_message(self, *_):
                    pass

            proxy = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.load_cert_chain(temp / 'cert.pem', temp / 'key.pem')
            proxy.socket = context.wrap_socket(proxy.socket, server_side=True)
            threading.Thread(target=proxy.serve_forever, daemon=True).start()
            yield temp, proxy.server_port, wallet
        finally:
            if proxy:
                proxy.shutdown()
                proxy.server_close()
            server.terminate()
            server.wait(timeout=10)

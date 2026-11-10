#!/usr/bin/env python3
"""Exercise the shipped Java API/solver against isolated real SQLite + HTTPS."""
import http.client
import http.server
import json
import os
from pathlib import Path
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time

root = Path(__file__).resolve().parents[3]
android = root / 'apps/pengminer-android'
json_jar = Path(sys.argv[1]).resolve()
def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]

with tempfile.TemporaryDirectory() as directory:
    temp = Path(directory)
    backend_port = port()
    env = dict(os.environ, PENGCHAIN_DB=str(temp / 'test.sqlite'), HOST='127.0.0.1', PORT=str(backend_port))
    server = subprocess.Popen(['node', 'dist/index.js'], cwd=root / 'server', env=env, stdout=subprocess.DEVNULL)
    try:
        for attempt in range(100):
            try:
                connection = http.client.HTTPConnection('127.0.0.1', backend_port, timeout=1)
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
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', str(temp / 'key.pem'), '-out', str(temp / 'cert.pem'), '-days', '1', '-subj', '/CN=localhost', '-addext', 'subjectAltName=DNS:localhost'], check=True, capture_output=True)
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
        source = android / 'app/src/main/java/org/pikastudio/pengminer'
        subprocess.run(['javac', '-cp', str(json_jar), '-d', str(temp), str(source / 'Pca10.java'), str(source / 'PengApi.java'), str(android / 'tests/ServerProof.java')], check=True)
        subprocess.run(['java', '-cp', str(temp) + os.pathsep + str(json_jar), 'org.pikastudio.pengminer.ServerProof', f'https://localhost:{proxy.server_port}', str(temp / 'cert.pem'), wallet], check=True)
        proxy.shutdown()
    finally:
        server.terminate()
        server.wait(timeout=10)

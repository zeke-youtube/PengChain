"""Release CLI protocol smoke test with an independent Python PoW verifier."""
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import threading
import time
import unittest

BINARY = Path(__file__).resolve().parents[1] / 'target/release/pengwallet'


class MiningSmoke(unittest.TestCase):
    def test_real_nonce_submission_and_rejection(self):
        state = {'reject': False, 'submissions': 0, 'requests': 0, 'expire_next': False}
        job = dict(jobId='job', tokenId='PENG', walletId='wallet',
                   challenge=base64.urlsafe_b64encode(bytes(32)).decode().rstrip('='),
                   difficulty=3, expiresAt=int(time.time() * 1000) + 60000)

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def respond(self, status, body):
                data = json.dumps(body).encode()
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                assert self.path == '/blockchain/api/mining/job?symbol=PENG'
                assert self.headers['Cookie'] == 'peng_session=test'
                state['requests'] += 1
                if state['expire_next']:
                    state['expire_next'] = False
                    self.respond(200, dict(job, expiresAt=0))
                else:
                    self.respond(200, job)

            def do_POST(self):
                assert self.path == '/blockchain/api/mining/submit'
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                assert body['jobId'] == job['jobId']
                assert isinstance(body['nonce'], str) and body['nonce'].isdigit()
                fields = [bytes([1]), b'pengchain-v1', b'job', b'PENG', b'wallet', bytes(32)]
                digest = b''.join(struct.pack('>I', len(f)) + f for f in fields)
                digest += struct.pack('>Q', int(body['nonce']))
                for _ in range(10):
                    digest = hashlib.sha256(digest).digest()
                assert digest.hex().startswith('000'), 'invalid proof of work'
                state['submissions'] += 1
                if state['reject']:
                    self.respond(400, {'error': 'stale or already used job'})
                else:
                    self.respond(200, {'accepted': True, 'reward': 10, 'digest': digest.hex()})

        with tempfile.TemporaryDirectory() as directory:
            session = Path(directory) / 'PengChain'
            session.mkdir()
            (session / 'session.json').write_text(json.dumps(dict(wallet_id='wallet', cookie='peng_session=test')))
            with http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler) as server:
                thread = threading.Thread(target=server.serve_forever, daemon=True)
                thread.start()
                try:
                    env = dict(os.environ, XDG_DATA_HOME=directory, NETWORK_ID='pengchain-v1')
                    command = [str(BINARY), '--server', f'http://127.0.0.1:{server.server_port}', 'mine', 'PENG', '--once']
                    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=20)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn('Reward accepted', result.stdout)
                    self.assertIn('+10 PENG base units', result.stdout)
                    self.assertNotIn('\x1b', result.stdout)
                    result = subprocess.run(command + ['--json'], env=env, capture_output=True, text=True, timeout=20)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertTrue(json.loads(result.stdout)['accepted'])
                    self.assertEqual(result.stderr, '')
                    requests = state['requests']
                    state['expire_next'] = True
                    result = subprocess.run(command + ['--json'], env=env, capture_output=True, text=True, timeout=20)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertTrue(json.loads(result.stdout)['accepted'])
                    self.assertEqual(state['requests'], requests + 2)
                    state['reject'] = True
                    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=20)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn('stale or already used job', result.stderr)
                    self.assertEqual(state['submissions'], 4)
                finally:
                    server.shutdown()
                    thread.join()


if __name__ == '__main__':
    unittest.main()

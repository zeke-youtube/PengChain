"""Real CLI subprocess protocol, private-session pipes, and EOF cancellation."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / 'client/target/release' / ('pengwallet.exe' if os.name == 'nt' else 'pengwallet')

class EngineSmoke(unittest.TestCase):
    def test_real_engine_events_and_private_session(self):
        with tempfile.TemporaryDirectory() as directory:
            with socket.socket() as listener:
                listener.bind(('127.0.0.1', 0))
                port = listener.getsockname()[1]
            address = f'http://127.0.0.1:{port}'
            env = dict(os.environ, HOST='127.0.0.1', PORT=str(port),
                       PENGCHAIN_DB=f'{directory}/ledger.sqlite', XDG_DATA_HOME=directory)
            server = subprocess.Popen(['node', 'dist/index.js'], cwd=ROOT / 'server',
                                      env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            def request(path, data=None, cookie=None):
                headers = {'Content-Type': 'application/json'}
                if cookie:
                    headers['Cookie'] = cookie
                response = opener.open(urllib.request.Request(address + path,
                    data=json.dumps(data).encode() if data is not None else None, headers=headers), timeout=5)
                with response:
                    return json.load(response), response.headers
            try:
                for _ in range(100):
                    try:
                        request('/health')
                        break
                    except OSError:
                        time.sleep(.05)
                wallet, _ = request('/blockchain/api/wallet/create', {'password': 'engine-smoke-password'})
                _, headers = request('/blockchain/api/wallet/login', {'walletId': wallet['walletId'], 'password': 'engine-smoke-password'})
                cookie = headers['set-cookie'].split(';')[0]
                config = dict(command='mine', version=1, server=address, wallet_id=wallet['walletId'],
                              cookie=cookie, token='PENG', network='pengchain-v1', gpu=False,
                              gpu_device=None, threads=1, intensity=100)
                command = [str(CLI), 'engine', '--json']
                self.assertNotIn(cookie, ' '.join(command))
                process = subprocess.Popen(command, env=env, text=True, stdin=subprocess.PIPE,
                                           stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                frames = []
                try:
                    process.stdin.write(json.dumps(config) + '\n')
                    process.stdin.flush()
                    # A reader thread provides a timeout even if a broken engine hangs.
                    import queue
                    import threading
                    output = queue.Queue()
                    def read_frames():
                        for line in process.stdout:
                            output.put(json.loads(line))
                    reader = threading.Thread(target=read_frames, daemon=True)
                    reader.start()
                    while sum(frame['event'] == 'reward_accepted' for frame in frames) < 2:
                        frames.append(output.get(timeout=15))
                    process.stdin.write('{"command":"stop"}\n')
                    process.stdin.flush()
                    process.wait(timeout=5)
                    reader.join(timeout=2)
                    while not output.empty():
                        frames.append(output.get_nowait())
                    self.assertEqual(process.returncode, 0)
                    self.assertTrue(all(frame['protocol_version'] == 1 for frame in frames))
                    self.assertEqual(frames[0]['event'], 'ready')
                    self.assertEqual(frames[-1]['event'], 'stopped')
                    self.assertGreater(frames[-1]['hashes'], 0)
                    for event in ['started', 'job_received', 'solution_found', 'hashrate']:
                        self.assertTrue(any(frame['event'] == event for frame in frames))
                    self.assertNotIn(cookie, json.dumps(frames))
                    self.assertNotIn('engine-smoke-password', json.dumps(frames))
                    self.assertFalse((Path(directory) / 'PengChain/session.json').exists())
                    balances, _ = request('/blockchain/api/wallet/balance', cookie=cookie)
                    receipts = [frame for frame in frames if frame['event'] == 'reward_accepted']
                    self.assertGreaterEqual(len(receipts), 2)
                    self.assertTrue(all(5 <= receipt['reward'] <= 15 for receipt in receipts))
                    self.assertEqual(balances[0]['amount'], sum(receipt['reward'] for receipt in receipts))
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait()
                    process.stdin.close()
                    process.stdout.close()
                    process.stderr.close()
                request('/blockchain/api/token', dict(name='EOF cancellation',symbol='HARD',mineable=True,maxSupply=100,difficulty=64), cookie)
                config['token'] = 'HARD'
                process = subprocess.Popen(command, env=env, text=True, stdin=subprocess.PIPE,
                                           stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                try:
                    process.stdin.write(json.dumps(config) + '\n')
                    process.stdin.flush()
                    time.sleep(.5)
                    process.stdin.close()
                    process.stdin = None
                    stdout, stderr = process.communicate(timeout=5)
                    self.assertEqual(process.returncode, 0, stderr)
                    frames = [json.loads(line) for line in stdout.splitlines()]
                    self.assertEqual(frames[-1]['event'], 'stopped')
                    self.assertFalse(any(frame['event'] == 'reward_accepted' for frame in frames))
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait()
            finally:
                server.terminate()
                server.wait(timeout=5)
                server.stderr.close()

if __name__ == '__main__':
    unittest.main()

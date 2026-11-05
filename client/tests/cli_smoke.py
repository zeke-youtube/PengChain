"""Exercise the release CLI against a real, isolated PengChain server.

Build server/dist and client/target/release/pengwallet before running this test.
Node must be on PATH. Temporary wallets, sessions, and databases are removed.
"""
import json
import os
import re
from pathlib import Path
import socket
import signal
import subprocess
import tempfile
import time
import unittest
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / 'client/target/release/pengwallet'


class CliSmoke(unittest.TestCase):
    def test_wallet_workflow_and_readable_output(self):
        with tempfile.TemporaryDirectory() as directory:
            with socket.socket() as listener:
                listener.bind(('127.0.0.1', 0))
                port = listener.getsockname()[1]
            address = f'http://127.0.0.1:{port}'
            env = dict(os.environ, PENGCHAIN_DB=f'{directory}/ledger.sqlite',
                       HOST='127.0.0.1', PORT=str(port), NETWORK_ID='pengchain-v1',
                       XDG_DATA_HOME=directory, NO_COLOR='1')
            server = subprocess.Popen(['node', 'dist/index.js'], cwd=ROOT / 'server',
                                      env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            try:
                opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                for _ in range(100):
                    if server.poll() is not None:
                        self.fail(server.stderr.read().decode())
                    try:
                        with opener.open(address + '/health', timeout=1) as response:
                            self.assertTrue(json.load(response)['ok'])
                        break
                    except OSError:
                        time.sleep(0.05)
                else:
                    self.fail('Server never became ready')

                def run(*arguments, ok=True, session_home=None):
                    result = subprocess.run([str(BINARY), '--server', address, *arguments],
                                            env=dict(env, XDG_DATA_HOME=session_home) if session_home else env, text=True, capture_output=True, timeout=30)
                    self.assertEqual(result.returncode == 0, ok, result.stderr)
                    self.assertNotIn('\x1b', result.stdout + result.stderr)
                    self.assertNotIn('peng_session=', result.stdout + result.stderr)
                    self.assertNotIn('test-password-123', result.stdout + result.stderr)
                    return result

                run('balance', ok=False)
                created = json.loads(run('create', 'test-password-123', '--json').stdout)
                wallet = created['walletId']
                self.assertTrue(created['signedIn'])
                self.assertEqual(json.loads(run('balance', '--json').stdout), [])
                session_path = Path(directory) / 'PengChain/session.json'
                self.assertEqual(json.loads(session_path.read_text())['wallet_id'], wallet)
                created_screen = run('create', 'test-password-123').stdout
                self.assertIn("Wallet created · you're signed in", created_screen)
                self.assertIn('Next: pengwallet balance', created_screen)
                recipient = json.loads(session_path.read_text())['wallet_id']
                self.assertNotEqual(recipient, wallet)
                self.assertIn(recipient, created_screen)
                self.assertIn('No balances yet', run('balance').stdout)
                # Creation must preserve the new ID if saving its login fails.
                recovery_home = Path(directory) / 'recovery'
                blocked_session = recovery_home / 'PengChain/session.json'
                blocked_session.mkdir(parents=True)
                failed_create = run('create', 'test-password-123', '--json', ok=False,
                                    session_home=str(recovery_home))
                error = json.loads(failed_create.stderr)['error']
                self.assertIn('was created, but automatic sign-in failed', error)
                self.assertIn('Do not create it again', error)
                recovery_wallet = re.search(r'PENG-[A-F0-9]{8}', error).group()
                blocked_session.rmdir()
                run('login', recovery_wallet, 'test-password-123', session_home=str(recovery_home))
                self.assertEqual(json.loads(blocked_session.read_text())['wallet_id'], recovery_wallet)
                failure = run('login', wallet, 'incorrect-password', ok=False)
                self.assertIn('Authentication failed', failure.stderr)
                self.assertNotIn('{', failure.stderr)
                self.assertIn("You're signed in", run('login', wallet, 'test-password-123').stdout)
                self.assertIn('No balances yet', run('balance').stdout)
                tokens = run('token-search', 'PengChain').stdout
                self.assertIn('PENG · PengChain', tokens)
                self.assertIn('CPU mining', tokens)
                self.assertIn('Uncapped', tokens)
                self.assertNotIn('{', tokens)
                self.assertIn('No tokens found', run('token-search', 'nothing & nowhere').stdout)
                self.assertEqual(json.loads(run('token-search', 'nothing & nowhere', '--json').stdout), [])
                reward = run('mine', 'PENG', '--once')
                self.assertIn('Reward accepted', reward.stdout)
                self.assertIn('+10 PENG base units', reward.stdout)
                self.assertIn('Searching for a valid proof', reward.stderr)
                self.assertIn('PENG', run('balance').stdout)
                receipt = run('send', 'PENG', recipient, '3').stdout
                self.assertIn('Transfer complete', receipt)
                self.assertIn('3 base units', receipt)
                self.assertIn(recipient, receipt)
                self.assertIn('Receipt', receipt)
                balance = run('balance', '--json')
                self.assertEqual(balance.stderr, '')
                self.assertEqual(json.loads(balance.stdout)[0]['amount'], 7)
                rejection = run('send', 'PENG', recipient, '999', ok=False)
                self.assertIn('insufficient balance', rejection.stderr)
                rejection = run('send', 'PENG', recipient, '999', '--json', ok=False)
                self.assertIn('insufficient balance', json.loads(rejection.stderr)['error'])
                # An invalid GPU index must fail, never silently mine on the CPU.
                unavailable = run('mine', 'PENG', '--gpu', '--gpu-device', '9999', '--once', ok=False)
                self.assertIn('GPU mining unavailable', unavailable.stderr)
                self.assertNotIn('Reward accepted', unavailable.stdout)
                run('mine', 'PENG', '--gpu-device', '0', '--once', ok=False)
                self.assertEqual(json.loads(run('balance', '--json').stdout)[0]['amount'], 7)
                repeated = run('mine', 'PENG', '--jobs', '3', '--json')
                receipts = [json.loads(line) for line in repeated.stdout.splitlines()]
                self.assertEqual(len(receipts), 3)
                self.assertTrue(all(receipt['accepted'] for receipt in receipts))
                self.assertEqual(sum(receipt['reward'] for receipt in receipts), 30)
                self.assertEqual(json.loads(run('balance', '--json').stdout)[0]['amount'], 37)
                run('mine', 'PENG', '--jobs', '0', ok=False)
                run('mine', 'PENG', '--once', '--jobs', '2', ok=False)

                # Optional physical-GPU acceptance test. This is never enabled to
                # disguise a software OpenCL device as an actual GPU.
                if os.environ.get('PENGCHAIN_TEST_GPU') == '1':
                    gpu_receipts = run('mine', 'PENG', '--gpu', '--jobs', '3', '--json')
                    receipts = [json.loads(line) for line in gpu_receipts.stdout.splitlines()]
                    self.assertEqual(len(receipts), 3)
                    self.assertTrue(all(receipt['accepted'] for receipt in receipts))
                    self.assertEqual(json.loads(run('balance', '--json').stdout)[0]['amount'], 67)

                # Verify default continuous mode, then Ctrl+C during the inter-job wait.
                log_path = Path(directory) / 'continuous.log'
                with log_path.open('w') as output:
                    miner = subprocess.Popen([str(BINARY), '--server', address, 'mine', 'PENG', '--json'],
                                             env=env, stdout=output, stderr=subprocess.PIPE, text=True)
                    try:
                        deadline = time.monotonic() + 10
                        while not log_path.read_text().strip() and time.monotonic() < deadline:
                            time.sleep(0.02)
                        self.assertTrue(log_path.read_text().strip(), 'No reward from continuous miner')
                        miner.send_signal(signal.SIGINT)
                        _, errors = miner.communicate(timeout=3)
                        self.assertEqual(miner.returncode, 0, errors)
                        self.assertEqual(errors, '')
                    finally:
                        if miner.poll() is None:
                            miner.kill()
                            miner.wait()
                        miner.stderr.close()
                self.assertTrue(all(json.loads(line)['accepted'] for line in log_path.read_text().splitlines()))

                # An effectively unsolvable job exercises Ctrl+C inside the CPU loop.
                cookie = json.loads((Path(directory) / 'PengChain/session.json').read_text())['cookie']
                request = urllib.request.Request(address + '/blockchain/api/token',
                    data=json.dumps(dict(name='Cancellation test', symbol='HARD', mineable=True,
                                         maxSupply=100, difficulty=64)).encode(),
                    headers={'Content-Type': 'application/json', 'Cookie': cookie})
                with opener.open(request) as response:
                    self.assertEqual(response.status, 200)
                progress_path = Path(directory) / 'progress.log'
                with progress_path.open('w') as progress:
                    miner = subprocess.Popen([str(BINARY), '--server', address, 'mine', 'HARD'],
                                             env=env, stdout=subprocess.PIPE, stderr=progress, text=True)
                    try:
                        deadline = time.monotonic() + 5
                        while 'Searching for a valid proof' not in progress_path.read_text() and time.monotonic() < deadline:
                            time.sleep(0.02)
                        self.assertIn('Searching for a valid proof', progress_path.read_text())
                        miner.send_signal(signal.SIGINT)
                        summary, _ = miner.communicate(timeout=3)
                        self.assertEqual(miner.returncode, 0)
                        self.assertIn('Mining stopped', summary)
                        self.assertIn('0 HARD base units', summary)
                    finally:
                        if miner.poll() is None:
                            miner.kill()
                            miner.wait()
                        miner.stdout.close()
                self.assertIn('Signed out', run('logout').stdout)
                self.assertFalse((Path(directory) / 'PengChain/session.json').exists())
                run('balance', ok=False)
                print('\nExample mining receipt:\n' + reward.stdout)
            finally:
                server.terminate()
                server.wait(timeout=5)
                server.stderr.close()


if __name__ == '__main__':
    unittest.main()

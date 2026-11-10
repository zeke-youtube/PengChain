#!/usr/bin/env python3
"""Real native mining, then a fresh Android process restores the saved login."""
import base64
import os
from pathlib import Path
import subprocess
from server_fixture import fixture

sdk = os.environ.get('ANDROID_HOME') or os.environ.get('ANDROID_SDK_ROOT')
adb = str(Path(sdk) / 'platform-tools/adb') if sdk else 'adb'
with fixture() as (temp, port, wallet):
    certificate = base64.b64encode((temp / 'cert.pem').read_bytes()).decode()
    for method in ['nativeMiningAndSavedSession', 'restoreAfterProcessRestartAndSignOut']:
        subprocess.run([adb, 'shell', 'am', 'force-stop', 'org.pikastudio.pengminer'], check=True)
        command = [adb, 'shell', 'am', 'instrument', '-w', '-r', '-e', 'class',
                   'org.pikastudio.pengminer.WorkspaceTest#' + method,
                   '-e', 'fixtureServer', f'https://10.0.2.2:{port}',
                   '-e', 'fixtureWallet', wallet,
                   '-e', 'fixtureCertificate', certificate,
                   'org.pikastudio.pengminer.test/androidx.test.runner.AndroidJUnitRunner']
        result = subprocess.run(command, check=True, text=True, capture_output=True, timeout=150)
        print(result.stdout)
        if 'OK (1 test)' not in result.stdout or 'FAILURES' in result.stdout or 'Process crashed' in result.stdout:
            raise RuntimeError('Android native mining/session test did not pass: ' + result.stderr)
print('PASS: real Android mining, acknowledged balances, force-stop/restart login restoration, and sign-out revocation')

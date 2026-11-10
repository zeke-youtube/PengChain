#!/usr/bin/env python3
"""Run the shipped Java transport and solver against real HTTPS + SQLite."""
import os
from pathlib import Path
import subprocess
import sys
from server_fixture import ROOT, fixture

android = ROOT / 'apps/pengminer-android'
json_jar = Path(sys.argv[1]).resolve()
with fixture() as (temp, port, wallet):
    source = android / 'app/src/main/java/org/pikastudio/pengminer'
    subprocess.run(['javac', '-cp', str(json_jar), '-d', str(temp), str(source / 'Pca10.java'), str(source / 'PengApi.java'), str(android / 'tests/ServerProof.java')], check=True)
    subprocess.run(['java', '-cp', str(temp) + os.pathsep + str(json_jar), 'org.pikastudio.pengminer.ServerProof', f'https://localhost:{port}', str(temp / 'cert.pem'), wallet], check=True)

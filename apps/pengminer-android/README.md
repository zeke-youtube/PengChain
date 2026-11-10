# PengMiner Android

Native Android 8+ (API 26+) PENG CPU miner. The app logs into an existing PengChain
wallet over HTTPS, requests real PENG jobs, computes ten SHA-256 rounds over the
canonical v1 input, and submits decimal uint64 nonces. Only acknowledged server
receipts count as earnings. No simulated hashes or rewards, CLI subprocess, WebView,
GPU backend, wallet keys, or changes to the ledger protocol are involved.

## Build and install

Install JDK 17 and Android SDK platform 35. Set `ANDROID_HOME` to the SDK directory,
or set `sdk.dir` in an untracked `local.properties` file. Then:

```sh
cd apps/pengminer-android
./gradlew testDebugUnitTest lintDebug assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Windows uses `gradlew.bat`. The Gradle 8.9 wrapper is checked in with its official
SHA-256 pinned. CI builds the APK and uploads `PengMiner-Android-debug`. This is a
sideloadable debug-signed APK; a distribution release needs your own signing key.

Open PengMiner, enter the HTTPS server origin, wallet ID, password, and network ID
(default `pengchain-v1`). Choose CPU intensity and tap **Sign in & start mining**.
Allow notifications to keep the Stop action visible in the notification drawer.
The default server is `https://pengchain.pikastudio.org`. Wallet creation and server
administration remain in the existing wallet/server tools.

One CPU worker performs real hashing at 25%, 50%, or 100% duty-cycle targets. These
are scheduling settings, not measured battery/power limits. Average H/s includes
network waits. Earnings and balances are integer base units. Counters reset on
each explicit start; balances come from the server.

Mining continues in a foreground service while the screen is closed. Stop from
the app or notification. A request already submitted when Stop is tapped may still
be accepted; the worker reads its acknowledgement before finishing. Connection
and read timeouts are 10 and 15 seconds. Network errors, rejection, or malformed
responses end the session; a proof with an uncertain outcome is never replayed.
Expired unsolved jobs are replaced. Android may terminate the process; mining never
automatically restarts after process death, reboot, or a login failure.

The app stops at Android's severe thermal level on Android 10+. Android 8/9 do not
provide that API; monitor temperature yourself. No wake lock is acquired, so the
OS can suspend work while sleeping. The app uses a `specialUse` foreground service
for explicit CPU mining. Store submission and device/vendor power policies require
separate review; no Play Store eligibility is claimed.

Passwords and session cookies exist only in process memory and are never saved,
logged, put in notifications, or sent to the UI. Sign in again after stopping.
HTTPS certificate verification is mandatory, redirects are disabled, credential
URLs and non-origin server URLs are rejected, cleartext traffic and backups are
disabled, and the mining service is not exported. No autostart or background-start
receiver exists.

## Verification

Unit tests check the same exact PCA-10 vectors as Rust, nibble difficulty,
cancellation, expiry, and malformed challenges. An additional integration harness
compiles the shipped Java API and solver and runs them against a temporary real
server/SQLite database behind local HTTPS. The test trusts only its generated
fixture certificate, with no change to the shipped app's trust configuration:

```sh
npm --prefix server ci
npm --prefix server run build
curl -fsSL https://repo.maven.apache.org/maven2/org/json/json/20240303/json-20240303.jar -o /tmp/peng-json.jar
python3 apps/pengminer-android/tests/server-proof.py /tmp/peng-json.jar
```

Run those commands from the repository root with Java, Python 3, OpenSSL, and Node
22+ on PATH. The test checks unsafe origins, bad-password rejection, real proof
acceptance, reward range, wallet balance, and replay rejection. No live wallet or
production mining is used by tests.

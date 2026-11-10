# PengMiner Android

Android 8+ uses the desktop app's **same React screens, components, styles, and
command contract**, including Dashboard, Devices, Mining, Wallet, Settings, token
creation, measured hashrate charts, reward history, themes, and server audit logs.
The layout adapts to phone widths. CPU threads and 25/50/100% intensity are real;
OpenCL GPU and PPU mining are unavailable on Android and cannot be selected.

A restricted WebView renders the bundled UI. It cannot navigate to remote pages,
read local files, or make network requests. Only named native commands are exposed.
Java performs HTTPS authentication, encrypted session storage, PCA-10 hashing,
proof submission, counters, and the foreground service. Passwords and session
cookies are never returned to the UI. No mining or reward data is simulated.

## Build and install

Install Node 22+, JDK 17, Android SDK platform 35, and an up-to-date Android System
WebView on the device. Set `ANDROID_HOME` or untracked `local.properties` (`sdk.dir`).
From the repository root:

```sh
npm --prefix apps/pengminer ci
cd apps/pengminer-android
./gradlew testDebugUnitTest lintDebug assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Gradle builds and bundles the shared desktop UI automatically. Windows uses
`gradlew.bat`. Launch PengMiner, configure the HTTPS origin in Settings if needed,
and sign in to your existing wallet. Mining starts only when you press Start or
explicitly enable Start mining on launch. Start on launch validates the saved
session first; no boot receiver or automatic restart after process death exists.

## Login that stays saved

The session is encrypted with AES-GCM using a non-exportable Android Keystore key.
It survives Stop, activity/process/device restarts, and app updates signed with the
same key. The app never saves your password. Saved sessions are isolated by exact
server origin. Sign out revokes the server session and removes local ciphertext;
if offline, the local copy is still removed. Clearing app data or uninstalling
removes the saved login. Backups and device transfers of app data are disabled.

Deploy the updated PengChain server to support `rememberMe: true`: remembered
native sessions have no automatic expiry and remain valid until logout or
server-side revocation. Ordinary browser logins still expire after 24 hours.
Older servers continue to work, but the app shows a notice that they still expire
sessions. Deploying this APK alone cannot change the session policy of an older
server. The desktop app also requests remembered sessions.

## Mining behavior

The same canonical v1 encoding and ten SHA-256 rounds are used. Parallel workers
search separate uint64 nonce lanes, with decimal strings at the API boundary.
Only acknowledged receipts count as earnings. Balances and rewards are integer
base units; recent receipts are scoped to the wallet/server and retained locally.
Hashrate counts completed evaluations and includes network waits in its average.

A foreground service keeps Stop visible while mining. Closing the app keeps
mining when the corresponding setting is enabled; disabling it stops mining when
the app leaves the foreground. Stop cancels CPU work and waits for any already
submitted proof acknowledgement. Unknown submission outcomes are never replayed.
Network errors and rejected proofs stop the worker. Android can terminate the
service; it never automatically restarts. No wake lock is acquired.

Android 10+ stops at severe thermal status. Physical temperature/performance and
vendor power policies still need device validation. Notifications and OS battery
policies may affect background operation. No Play Store eligibility is claimed.

## Automatic GitHub releases

`PengMiner Windows` calls the Android workflow, builds both apps from the same
commit, and tests the server, shared frontend, CPU/CLI, desktop engine, and Android
UI/Keystore. Successful pushes to `main` automatically publish **build-RUN_ID**
with every APK/installer and `SHA256SUMS.txt`. Explicit manual runs also publish.
Pull requests build/test without publishing. An incomplete upload stays in draft;
reruns cannot replace an already published build with different binaries.

Optional stable release signing uses repository Actions secrets:

- `PENGMINER_ANDROID_KEYSTORE_BASE64`: base64 PKCS12/JKS, alias `pengminer`.
- `PENGMINER_ANDROID_KEYSTORE_PASSWORD`: the keystore/key password.

Without them, CI publishes a clearly named debug APK. A new CI debug signing key
can require uninstalling an older APK, which clears saved login. To preserve login
across future APK updates, configure the stable private signing key once. Do not
commit signing files or paste secret values into chat. The current connection
cannot administer repository secrets; configure them in GitHub Settings.

## Verification

```sh
# Repository root; Java, Node 22+, Python, and OpenSSL on PATH
npm --prefix server ci
npm --prefix server run build
npm --prefix server test
node --test scripts/publish-build.test.cjs
curl -fsSL https://repo.maven.apache.org/maven2/org/json/json/20240303/json-20240303.jar -o /tmp/peng-json.jar
python3 apps/pengminer-android/tests/server-proof.py /tmp/peng-json.jar

# Android device/emulator with System WebView installed
cd apps/pengminer-android
./gradlew connectedDebugAndroidTest
```

Tests include exact cross-language PCA-10 vectors, parallel proof correctness,
active-job cancellation, HTTPS proof acceptance/replay rejection, SQLite session
restart/revocation, encrypted Android login restore/clear/origin isolation, and
rendering all six desktop pages without horizontal overflow on a phone.

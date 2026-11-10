package org.pikastudio.pengminer;

import android.content.Context;
import android.content.Intent;
import android.os.SystemClock;
import android.util.AtomicFile;
import java.io.File;
import java.io.FileOutputStream;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Consumer;
import org.json.JSONArray;
import org.json.JSONObject;

/** Native backend for the same command/snapshot contract as the desktop app. */
final class MinerController {
  @android.annotation.SuppressLint(
      "StaticFieldLeak") // Holds the Application context, never an Activity.
  private static MinerController instance;

  static synchronized MinerController get(Context context) {
    if (instance == null) instance = new MinerController(context.getApplicationContext());
    return instance;
  }

  private final Context context;
  private final SessionVault vault;
  private final JSONObject state = new JSONObject();
  private JSONArray history = new JSONArray();
  private final AtomicBoolean cancel = new AtomicBoolean();
  private final AtomicLong hashes = new AtomicLong();
  private volatile PengApi api;
  private volatile Consumer<JSONObject> listener;
  private volatile CountDownLatch stopped = new CountDownLatch(0);
  private volatile boolean running;
  private final AtomicBoolean initialized = new AtomicBoolean();
  private final CountDownLatch initializationReady = new CountDownLatch(1);
  private long started;
  private long sampledAt;
  private long previousHashes;

  private MinerController(Context context) {
    this.context = context;
    vault = new SessionVault(context);
    try {
      JSONObject settings = read("settings.json", defaults());
      settings = validateSettings(settings);
      state
          .put("settings", settings)
          .put("running", false)
          .put("connected", false)
          .put("walletId", JSONObject.NULL)
          .put("persistent", false)
          .put("currentHashrate", 0)
          .put("averageHashrate", 0)
          .put("hashes", 0)
          .put("accepted", 0)
          .put("rejected", 0)
          .put("earnings", 0)
          .put("uptime", 0)
          .put("difficulty", JSONObject.NULL)
          .put("job", JSONObject.NULL)
          .put("deviceName", "Android CPU")
          .put("cpuCount", cpuCount())
          .put("balances", new JSONArray())
          .put("tokens", new JSONArray())
          .put("history", new JSONArray())
          .put("logs", new JSONArray())
          .put("auditLogs", new JSONArray())
          .put("error", JSONObject.NULL);
      File receipts = new File(context.getFilesDir(), "history.json");
      if (receipts.exists())
        history =
            new JSONArray(
                new String(Files.readAllBytes(receipts.toPath()), StandardCharsets.UTF_8));
    } catch (Exception error) {
      throw new IllegalStateException("Could not load workspace settings", error);
    }
  }

  static int cpuCount() {
    return Math.max(1, Runtime.getRuntime().availableProcessors());
  }

  static JSONObject defaults() throws Exception {
    return new JSONObject()
        .put("enginePath", "")
        .put("server", "https://pengchain.pikastudio.org")
        .put("token", "PENG")
        .put("network", "pengchain-v1")
        .put("device", "cpu")
        .put("cpuThreads", 1)
        .put("intensity", 25)
        .put("startOnLaunch", false)
        .put("minimizeToTray", true)
        .put("notifications", false)
        .put("theme", "dark");
  }

  static JSONObject validateSettings(JSONObject value) throws Exception {
    JSONObject settings = defaults();
    java.util.Iterator<String> keys = settings.keys();
    while (keys.hasNext()) {
      String key = keys.next();
      if (value.has(key)) settings.put(key, value.get(key));
    }
    String server = new PengApi(settings.getString("server").trim(), "").origin;
    int threads = settings.getInt("cpuThreads");
    int intensity = settings.getInt("intensity");
    String token = settings.getString("token").toUpperCase(java.util.Locale.ROOT);
    String network = settings.getString("network");
    if (threads < 1 || threads > cpuCount())
      throw new IllegalArgumentException("Select 1–" + cpuCount() + " CPU threads");
    if (intensity != 25 && intensity != 50 && intensity != 100)
      throw new IllegalArgumentException("Invalid intensity");
    if (!token.matches("[A-Z][A-Z0-9]{0,15}"))
      throw new IllegalArgumentException("Invalid token symbol");
    if (network.isEmpty() || network.length() > 128)
      throw new IllegalArgumentException("Invalid network ID");
    if (!"cpu".equals(settings.getString("device")))
      throw new IllegalArgumentException(
          "Android supports CPU mining; GPU and PPU mining are unavailable");
    if (!java.util.Arrays.asList("dark", "light", "system").contains(settings.getString("theme")))
      throw new IllegalArgumentException("Invalid theme");
    settings.put("server", server).put("token", token).put("enginePath", "");
    return settings;
  }

  synchronized JSONObject snapshot() throws Exception {
    return new JSONObject(state.toString());
  }

  synchronized JSONObject settings() throws Exception {
    return new JSONObject(state.getJSONObject("settings").toString());
  }

  boolean isRunning() {
    return running;
  }

  void listen(Consumer<JSONObject> callback) {
    listener = callback;
  }

  void unlisten(Consumer<JSONObject> callback) {
    if (listener == callback) listener = null;
  }

  Object command(String name, JSONObject args) throws Exception {
    initialize();
    switch (name) {
      case "snapshot":
        return snapshot();
      case "devices":
        return new JSONObject()
            .put("gpus", new JSONArray())
            .put(
                "warning",
                "Android CPU mining is available. OpenCL GPU and PPU backends are not available on"
                    + " Android.");
      case "save_settings":
        saveSettings(args.getJSONObject("settings"));
        break;
      case "login":
        login(args.getString("wallet"), args.getString("password"));
        break;
      case "logout":
        logout();
        break;
      case "refresh":
        refresh();
        break;
      case "start_mining":
        start();
        break;
      case "stop_mining":
        stopAndWait();
        break;
      case "create_token":
        requireSession().request("POST", "/token", args.getJSONObject("request"));
        log("token_created", "Custom token registered with the server.");
        refresh();
        break;
      default:
        throw new IllegalArgumentException("Unsupported native command");
    }
    return snapshot();
  }

  private void initialize() throws Exception {
    if (!initialized.compareAndSet(false, true)) {
      if (!initializationReady.await(90, TimeUnit.SECONDS)) {
        throw new IllegalStateException(
            "Session restoration is taking longer than expected. Try again.");
      }
      return;
    }
    try {
      String server = settings().getString("server");
      try {
        api = vault.load(server);
        if (api != null) {
          synchronized (this) {
            state.put("walletId", api.wallet).put("persistent", true);
            sessionNotice();
            filterHistory();
          }
          log("session_restored", "Saved login restored from Android Keystore.");
        }
      } catch (Exception error) {
        vault.clear(server);
        log("session_error", "Saved login could not be decrypted. Sign in again.");
      }
      try {
        refresh();
        if (api != null && !cancel.get() && settings().optBoolean("startOnLaunch")) start();
      } catch (Exception error) {
        fail(error);
      }
    } finally {
      initializationReady.countDown();
    }
  }

  private void login(String wallet, String password) throws Exception {
    if (running) throw new IllegalStateException("Stop mining before signing in");
    PengApi session = new PengApi(settings().getString("server"), wallet.trim());
    session.login(password);
    vault.save(session);
    api = session;
    synchronized (this) {
      state.put("walletId", session.wallet).put("persistent", true).put("error", JSONObject.NULL);
      sessionNotice();
      filterHistory();
    }
    log("login", "Signed in. Login is protected by Android Keystore until sign out or revocation.");
    refresh();
  }

  private void logout() throws Exception {
    stopAndWait();
    PengApi session = api;
    if (session != null) {
      // Always forget locally even if the server is offline.
      try {
        session.request("POST", "/wallet/logout", new JSONObject());
      } finally {
        clearSession();
      }
    }
    log("logout", "Saved login removed from this device.");
  }

  private synchronized void sessionNotice() throws Exception {
    state.put(
        "sessionNotice",
        api != null && !api.remembered
            ? "Login is saved securely, but this server still expires sessions after 24 hours."
                + " Update the PengChain server to enable lasting login."
            : JSONObject.NULL);
  }

  private void clearSession() throws Exception {
    String server = settings().getString("server");
    vault.clear(server);
    api = null;
    synchronized (this) {
      state
          .put("walletId", JSONObject.NULL)
          .put("persistent", false)
          .put("balances", new JSONArray())
          .put("history", new JSONArray())
          .put("sessionNotice", JSONObject.NULL);
    }
  }

  private void saveSettings(JSONObject value) throws Exception {
    if (running) throw new IllegalStateException("Stop mining before changing settings");
    JSONObject next = validateSettings(value);
    boolean changedServer = !next.getString("server").equals(settings().getString("server"));
    write("settings.json", next.toString());
    synchronized (this) {
      state.put("settings", next).put("error", JSONObject.NULL);
    }
    if (changedServer) {
      api = null;
      try {
        api = vault.load(next.getString("server"));
      } catch (Exception error) {
        vault.clear(next.getString("server"));
        log("session_error", "Saved login for this server could not be restored. Sign in again.");
      }
      synchronized (this) {
        state
            .put("walletId", api == null ? JSONObject.NULL : api.wallet)
            .put("persistent", api != null)
            .put("balances", new JSONArray())
            .put("connected", false)
            .put("auditLogs", new JSONArray());
        sessionNotice();
        filterHistory();
      }
    }
    log("settings_saved", "Workspace settings saved.");
  }

  private synchronized PengApi requireSession() {
    PengApi session = api;
    if (session == null
        || !session.origin.equals(state.optJSONObject("settings").optString("server")))
      throw new IllegalStateException("Sign in before mining or changing your wallet");
    return session;
  }

  private void refresh() throws Exception {
    PengApi session = api;
    PengApi connection =
        session == null ? new PengApi(settings().getString("server"), "") : session;
    try {
      JSONArray tokens = (JSONArray) connection.request("GET", "/token?limit=100", null);
      JSONArray balances =
          session == null
              ? new JSONArray()
              : (JSONArray) session.request("GET", "/wallet/balance", null);
      JSONArray audit =
          session == null
              ? new JSONArray()
              : (JSONArray) session.request("GET", "/logs?limit=100", null);
      synchronized (this) {
        state
            .put("tokens", tokens)
            .put("balances", balances)
            .put("auditLogs", audit)
            .put("connected", true)
            .put("error", JSONObject.NULL);
      }
    } catch (Exception error) {
      synchronized (this) {
        state.put("connected", false);
      }
      if (error instanceof PengApi.Failure && ((PengApi.Failure) error).status == 401) {
        cancel.set(true);
        clearSession();
      }
      throw error;
    }
    emit("network_refreshed");
  }

  private synchronized void start() throws Exception {
    if (running) throw new IllegalStateException("Mining is already running");
    requireSession();
    validateSettings(settings());
    running = true;
    cancel.set(false);
    hashes.set(0);
    started = sampledAt = SystemClock.elapsedRealtime();
    previousHashes = 0;
    stopped = new CountDownLatch(1);
    state
        .put("running", true)
        .put("hashes", 0)
        .put("accepted", 0)
        .put("rejected", 0)
        .put("earnings", 0)
        .put("uptime", 0)
        .put("currentHashrate", 0)
        .put("averageHashrate", 0)
        .put("job", JSONObject.NULL)
        .put("error", JSONObject.NULL);
    try {
      context.startForegroundService(new Intent(context, MiningService.class));
    } catch (RuntimeException error) {
      running = false;
      stopped.countDown();
      state.put("running", false);
      throw error;
    }
    log("mining_started", "Real PCA-10 CPU mining started.");
  }

  void requestStop() {
    cancel.set(true);
  }

  private void stopAndWait() throws Exception {
    cancel.set(true);
    if (!stopped.await(30, TimeUnit.SECONDS))
      throw new IllegalStateException(
          "Waiting for a submitted proof acknowledgement. Check your wallet balance before"
              + " restarting.");
  }

  void thermalStop() {
    cancel.set(true);
    try {
      synchronized (this) {
        state.put("error", "Device temperature is high. Mining stopped.");
      }
    } catch (Exception ignored) {
    }
  }

  void mine() {
    try {
      PengApi session = requireSession();
      JSONObject settings = settings();
      String token = settings.getString("token");
      synchronized (this) {
        state.put("deviceName", "Android CPU · " + settings.getInt("cpuThreads") + " thread(s)");
      }
      while (!cancel.get()) {
        JSONObject job =
            (JSONObject)
                session.request(
                    "GET", "/mining/job?symbol=" + URLEncoder.encode(token, "UTF-8"), null);
        if (!session.wallet.equals(job.getString("walletId"))
            || !token.equals(job.getString("symbol"))) {
          throw new IllegalStateException("Server returned a job for another wallet or token");
        }
        synchronized (this) {
          state.put("job", job).put("difficulty", job.getInt("difficulty"));
        }
        log("mining_job_received", "Received a server challenge for " + token + ".");
        String nonce =
            Pca10.solveParallel(
                Pca10.prefix(
                    settings.getString("network"),
                    job.getString("jobId"),
                    job.getString("tokenId"),
                    session.wallet,
                    job.getString("challenge"),
                    job.getInt("difficulty")),
                job.getInt("difficulty"),
                job.getLong("expiresAt"),
                cancel,
                hashes,
                settings.getInt("cpuThreads"),
                settings.getInt("intensity"));
        if (cancel.get()) break;
        if (nonce == null) continue;
        log("solution_found", "A real PCA-10 proof was found.");
        JSONObject receipt;
        try {
          receipt =
              (JSONObject)
                  session.request(
                      "POST",
                      "/mining/submit",
                      new JSONObject().put("jobId", job.getString("jobId")).put("nonce", nonce));
        } catch (PengApi.Failure error) {
          if (error.status == 400) record(session, token, job, nonce, false, 0);
          throw error;
        }
        if (!receipt.getBoolean("accepted"))
          throw new IllegalStateException("Proof was not accepted");
        long reward = receipt.getLong("reward");
        if (reward < 0) throw new IllegalStateException("Invalid server reward");
        record(session, token, job, nonce, true, reward);
        JSONArray balances = (JSONArray) session.request("GET", "/wallet/balance", null);
        synchronized (this) {
          state.put("balances", balances);
        }
      }
    } catch (Exception error) {
      if (error instanceof PengApi.Failure && ((PengApi.Failure) error).status == 401) {
        try {
          clearSession();
        } catch (Exception ignored) {
        }
      }
      fail(error);
    } finally {
      try {
        sample();
        synchronized (this) {
          state.put("running", false).put("currentHashrate", 0).put("job", JSONObject.NULL);
          running = false;
        }
        log("mining_stopped", "Mining stopped. Unknown submission outcomes are never replayed.");
      } catch (Exception ignored) {
        running = false;
      }
      stopped.countDown();
    }
  }

  private void record(
      PengApi session, String token, JSONObject job, String nonce, boolean accepted, long reward)
      throws Exception {
    synchronized (this) {
      String counter = accepted ? "accepted" : "rejected";
      state.put(counter, state.getLong(counter) + 1);
      if (accepted) state.put("earnings", Math.addExact(state.getLong("earnings"), reward));
      history.put(
          new JSONObject()
              .put("time", System.currentTimeMillis())
              .put("walletId", session.wallet)
              .put("server", session.origin)
              .put("token", token)
              .put("jobId", job.getString("jobId"))
              .put("nonce", nonce)
              .put("accepted", accepted)
              .put("reward", reward));
      history = last(history, 1000);
      write("history.json", history.toString());
      filterHistory();
    }
    log(
        accepted ? "reward_accepted" : "submission_rejected",
        accepted
            ? "Server accepted reward: " + reward + " " + token + " base units."
            : "Server rejected the proof. Mining stopped.");
  }

  synchronized void sample() throws Exception {
    if (started == 0) return;
    long now = SystemClock.elapsedRealtime();
    long count = hashes.get();
    double elapsed = Math.max(0.001, (now - started) / 1000.0);
    state
        .put("hashes", count)
        .put("uptime", elapsed)
        .put(
            "currentHashrate",
            (count - previousHashes) / Math.max(0.001, (now - sampledAt) / 1000.0))
        .put("averageHashrate", count / elapsed);
    previousHashes = count;
    sampledAt = now;
    emit("hashrate_updated");
  }

  void fail(Exception error) {
    try {
      String message =
          error instanceof java.io.IOException
              ? (running
                  ? "Connection lost. Mining stopped; check wallet balance before restarting."
                  : "Could not reach the PengChain server. Check your connection and refresh.")
              : error.getMessage();
      synchronized (this) {
        state.put("error", message);
        if (error instanceof java.io.IOException) state.put("connected", false);
      }
      log(running ? "mining_error" : "workspace_error", message);
    } catch (Exception ignored) {
    }
  }

  private synchronized void log(String kind, String message) throws Exception {
    JSONArray logs = state.getJSONArray("logs");
    logs.put(
        new JSONObject()
            .put("time", System.currentTimeMillis())
            .put("kind", kind)
            .put("message", message));
    state.put("logs", last(logs, 150));
    emit(kind);
  }

  private synchronized void emit(String kind) throws Exception {
    Consumer<JSONObject> callback = listener;
    if (callback != null)
      callback.accept(new JSONObject().put("kind", kind).put("snapshot", snapshot()));
  }

  private synchronized void filterHistory() throws Exception {
    JSONArray selected = new JSONArray();
    if (api != null)
      for (int i = 0; i < history.length(); i++) {
        JSONObject row = history.getJSONObject(i);
        if (api.wallet.equals(row.optString("walletId"))
            && api.origin.equals(row.optString("server"))) selected.put(row);
      }
    state.put("history", last(selected, 200));
  }

  private static JSONArray last(JSONArray rows, int limit) throws Exception {
    JSONArray result = new JSONArray();
    for (int i = Math.max(0, rows.length() - limit); i < rows.length(); i++)
      result.put(rows.get(i));
    return result;
  }

  private JSONObject read(String name, JSONObject fallback) throws Exception {
    AtomicFile file = new AtomicFile(new File(context.getFilesDir(), name));
    try {
      return new JSONObject(new String(file.readFully(), StandardCharsets.UTF_8));
    } catch (java.io.FileNotFoundException missing) {
      return fallback;
    }
  }

  private void write(String name, String value) throws Exception {
    AtomicFile file = new AtomicFile(new File(context.getFilesDir(), name));
    FileOutputStream output = file.startWrite();
    try {
      output.write(value.getBytes(StandardCharsets.UTF_8));
      file.finishWrite(output);
    } catch (Exception error) {
      file.failWrite(output);
      throw error;
    }
  }
}

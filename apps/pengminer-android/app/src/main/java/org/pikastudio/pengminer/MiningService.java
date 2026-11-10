package org.pikastudio.pengminer;

import android.app.*;
import android.content.*;
import android.os.*;
import org.json.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;

/** One explicit user-started CPU worker. No automatic restart or request replay. */
public final class MiningService extends Service {
    static final String STOP = "org.pikastudio.pengminer.STOP";
    private static final String CHANNEL = "mining";
    private static final int NOTIFICATION = 1;
    static volatile String status = "Sign in to start mining";
    static volatile boolean running;
    private final AtomicBoolean stop = new AtomicBoolean();
    private final AtomicLong hashes = new AtomicLong();
    private final ExecutorService executor = Executors.newSingleThreadExecutor();
    private final Handler handler = new Handler(Looper.getMainLooper());
    private long started;
    private long accepted;
    private long earnings;
    private long balance;
    private PowerManager power;
    private ThermalMonitor thermalMonitor;

    @android.annotation.TargetApi(29)
    private static final class ThermalMonitor implements AutoCloseable {
        private final PowerManager power;
        private final PowerManager.OnThermalStatusChangedListener listener;
        ThermalMonitor(PowerManager power, Runnable stop) {
            this.power = power;
            listener = level -> {
                if (level >= PowerManager.THERMAL_STATUS_SEVERE) stop.run();
            };
            power.addThermalStatusListener(listener);
        }
        @Override public void close() { power.removeThermalStatusListener(listener); }
    }

    @Override public void onCreate() {
        super.onCreate();
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel(CHANNEL, "Active mining", NotificationManager.IMPORTANCE_LOW));
        power = getSystemService(PowerManager.class);
        if (Build.VERSION.SDK_INT >= 29) {
            thermalMonitor = new ThermalMonitor(power, () -> {
                status = "Stopped because device temperature is high";
                stop.set(true);
            });
        }
    }

    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null || STOP.equals(intent.getAction())) {
            if (running) { stop.set(true); status = "Stopping; waiting for any submitted proof acknowledgement"; }
            else stopSelf();
            return START_NOT_STICKY;
        }
        if (running) return START_NOT_STICKY;
        stop.set(false);
        hashes.set(0);
        accepted = 0;
        earnings = 0;
        balance = 0;
        started = SystemClock.elapsedRealtime();
        running = true;
        status = "Signing in…";
        startForeground(NOTIFICATION, notification(status));
        String server = intent.getStringExtra("server");
        String wallet = intent.getStringExtra("wallet");
        String password = intent.getStringExtra("password");
        String network = intent.getStringExtra("network");
        int intensity = intent.getIntExtra("intensity", 25);
        // Remove credentials from the Intent retained by the framework.
        intent.removeExtra("password");
        executor.execute(() -> mine(server, wallet, password, network, intensity));
        handler.post(tick);
        return START_NOT_STICKY;
    }

    private void mine(String server, String wallet, String password, String network, int intensity) {
        try {
            PengApi api = new PengApi(server, wallet);
            api.login(password);
            password = null;
            while (!stop.get()) {
                if (Build.VERSION.SDK_INT >= 29 && power.getCurrentThermalStatus() >= PowerManager.THERMAL_STATUS_SEVERE) {
                    throw new IllegalStateException("Device temperature is high. Let it cool before restarting.");
                }
                JSONObject job = (JSONObject) api.request("GET", "/mining/job?symbol=PENG", null);
                if (!wallet.equals(job.getString("walletId")) || !"PENG".equals(job.getString("symbol"))) {
                    throw new IllegalStateException("Server returned a job for another wallet or token");
                }
                int difficulty = job.getInt("difficulty");
                byte[] prefix = Pca10.prefix(network, job.getString("jobId"), job.getString("tokenId"), wallet,
                        job.getString("challenge"), difficulty);
                status = "Mining PENG · difficulty " + difficulty;
                String nonce = Pca10.solve(prefix, difficulty, job.getLong("expiresAt"), stop, hashes, intensity);
                if (stop.get()) break;
                if (nonce == null) continue;
                // Submit once. An uncertain network outcome ends this session; never retry the proof.
                JSONObject receipt = (JSONObject) api.request("POST", "/mining/submit",
                        new JSONObject().put("jobId", job.getString("jobId")).put("nonce", nonce));
                if (!receipt.getBoolean("accepted")) throw new IllegalStateException("Proof was not accepted");
                long reward = receipt.getLong("reward");
                if (reward < 0) throw new IllegalStateException("Invalid server reward");
                earnings = Math.addExact(earnings, reward);
                accepted++;
                JSONArray balances = (JSONArray) api.request("GET", "/wallet/balance", null);
                balance = 0;
                for (int i = 0; i < balances.length(); i++) {
                    JSONObject row = balances.getJSONObject(i);
                    if ("PENG".equals(row.getString("symbol"))) balance = row.getLong("amount");
                }
                status = "Accepted " + accepted + " · earned " + earnings + " · balance " + balance + " base units";
            }
            if (!status.startsWith("Stopped because")) status = "Stopped · " + accepted + " accepted · " + earnings + " base units earned";
        } catch (Exception error) {
            status = error instanceof java.io.IOException
                    ? "Network error. Stopped; check wallet balance before restarting (a submitted proof may have been accepted)."
                    : "Stopped: " + error.getMessage();
        } finally {
            handler.post(() -> {
                handler.removeCallbacks(tick);
                running = false;
                stopForeground(STOP_FOREGROUND_REMOVE);
                stopSelf();
            });
        }
    }

    private final Runnable tick = new Runnable() {
        @Override public void run() {
            if (!running) return;
            double seconds = Math.max(0.001, (SystemClock.elapsedRealtime() - started) / 1000.0);
            String summary = status + "\n" + accepted + " accepted · " + earnings + " earned · " + balance + " balance (base units)\n" + hashes.get() + " hashes · "
                    + String.format(java.util.Locale.ROOT, "%.1f", hashes.get() / seconds) + " H/s average";
            getSystemService(NotificationManager.class).notify(NOTIFICATION, notification(summary));
            MainActivity.miningSummary = summary;
            handler.postDelayed(this, 1000);
        }
    };

    private Notification notification(String text) {
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE);
        PendingIntent cancel = PendingIntent.getService(this, 1, new Intent(this, MiningService.class).setAction(STOP), PendingIntent.FLAG_IMMUTABLE);
        return new Notification.Builder(this, CHANNEL)
                .setSmallIcon(android.R.drawable.ic_media_play).setContentTitle("PengMiner · CPU mining")
                .setContentText(text).setStyle(new Notification.BigTextStyle().bigText(text))
                .setContentIntent(open).setOngoing(true).addAction(new Notification.Action.Builder(
                        null, "Stop mining", cancel).build()).build();
    }

    @Override public void onDestroy() {
        stop.set(true);
        handler.removeCallbacks(tick);
        if (Build.VERSION.SDK_INT >= 29 && thermalMonitor != null) thermalMonitor.close();
        executor.shutdown();
        super.onDestroy();
    }
    @Override public IBinder onBind(Intent intent) { return null; }
}

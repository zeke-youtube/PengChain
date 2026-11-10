package org.pikastudio.pengminer;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.json.JSONObject;

/** Android owns the foreground lifetime; the shared UI never performs mining. */
public final class MiningService extends Service {
  static final String STOP = "org.pikastudio.pengminer.STOP";
  private static final String CHANNEL = "mining";
  private final Handler handler = new Handler(Looper.getMainLooper());
  private final ExecutorService worker = Executors.newSingleThreadExecutor();
  private MinerController controller;
  private ThermalMonitor thermal;
  private boolean started;
  private int ticks;

  @android.annotation.TargetApi(29)
  private static final class ThermalMonitor implements AutoCloseable {
    private final PowerManager power;
    private final PowerManager.OnThermalStatusChangedListener listener;

    ThermalMonitor(PowerManager power, Runnable stop) {
      this.power = power;
      listener =
          level -> {
            if (level >= PowerManager.THERMAL_STATUS_SEVERE) stop.run();
          };
      power.addThermalStatusListener(listener);
      if (power.getCurrentThermalStatus() >= PowerManager.THERMAL_STATUS_SEVERE) stop.run();
    }

    @Override
    public void close() {
      power.removeThermalStatusListener(listener);
    }
  }

  @Override
  public void onCreate() {
    super.onCreate();
    controller = MinerController.get(this);
    getSystemService(NotificationManager.class)
        .createNotificationChannel(
            new NotificationChannel(CHANNEL, "Active mining", NotificationManager.IMPORTANCE_LOW));
  }

  @Override
  public int onStartCommand(Intent intent, int flags, int startId) {
    if (intent == null || STOP.equals(intent.getAction())) {
      controller.requestStop();
      if (!started) stopSelf();
      return START_NOT_STICKY;
    }
    if (started) return START_NOT_STICKY;
    if (!controller.isRunning()) {
      stopSelf();
      return START_NOT_STICKY;
    }
    started = true;
    startForeground(1, notification("Starting real PCA-10 mining…"));
    if (Build.VERSION.SDK_INT >= 29)
      thermal = new ThermalMonitor(getSystemService(PowerManager.class), controller::thermalStop);
    worker.execute(
        () -> {
          controller.mine();
          handler.post(
              () -> {
                handler.removeCallbacks(tick);
                stopForeground(STOP_FOREGROUND_REMOVE);
                stopSelf();
              });
        });
    handler.post(tick);
    return START_NOT_STICKY;
  }

  private final Runnable tick =
      new Runnable() {
        @Override
        public void run() {
          if (!controller.isRunning()) return;
          try {
            controller.sample();
            if (ticks++ % 4 == 0) {
              JSONObject state = controller.snapshot();
              String text =
                  state.getLong("accepted")
                      + " accepted · "
                      + state.getLong("earnings")
                      + " "
                      + state.getJSONObject("settings").getString("token")
                      + " earned · "
                      + Math.round(state.getDouble("currentHashrate"))
                      + " H/s";
              getSystemService(NotificationManager.class).notify(1, notification(text));
            }
          } catch (Exception error) {
            controller.fail(error);
            controller.requestStop();
          }
          handler.postDelayed(this, 500);
        }
      };

  private Notification notification(String text) {
    PendingIntent open =
        PendingIntent.getActivity(
            this,
            0,
            new Intent(this, MainActivity.class)
                .addFlags(
                    Intent.FLAG_ACTIVITY_NEW_TASK
                        | Intent.FLAG_ACTIVITY_CLEAR_TOP
                        | Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE);
    PendingIntent stop =
        PendingIntent.getService(
            this,
            1,
            new Intent(this, MiningService.class).setAction(STOP),
            PendingIntent.FLAG_IMMUTABLE);
    return new Notification.Builder(this, CHANNEL)
        .setSmallIcon(android.R.drawable.ic_media_play)
        .setContentTitle("PengMiner · CPU mining")
        .setContentText(text)
        .setContentIntent(open)
        .setOngoing(true)
        .addAction(new Notification.Action.Builder(null, "Stop mining", stop).build())
        .build();
  }

  @Override
  public void onDestroy() {
    controller.requestStop();
    handler.removeCallbacks(tick);
    if (Build.VERSION.SDK_INT >= 29 && thermal != null) thermal.close();
    worker.shutdown();
    super.onDestroy();
  }

  @Override
  public IBinder onBind(Intent intent) {
    return null;
  }
}

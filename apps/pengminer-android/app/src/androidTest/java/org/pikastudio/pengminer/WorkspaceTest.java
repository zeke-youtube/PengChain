package org.pikastudio.pengminer;

import static org.junit.Assert.*;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.webkit.WebView;
import android.widget.FrameLayout;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import org.json.JSONObject;
import org.junit.Test;
import org.junit.runner.RunWith;

@RunWith(AndroidJUnit4.class)
public class WorkspaceTest {
  @Test
  public void encryptedLoginRestoresAndClearsWithoutAPassword() throws Exception {
    Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
    PengApi api = new PengApi("https://vault-test.invalid", "PENG-TEST");
    api.restoreCookie("peng_session=test-session-secret");
    SessionVault first = new SessionVault(context);
    try {
      first.save(api);
      String encrypted =
          context
              .getSharedPreferences("encrypted-sessions", Context.MODE_PRIVATE)
              .getString(api.origin, "");
      assertFalse(encrypted.contains(api.wallet));
      assertFalse(encrypted.contains(api.sessionCookie()));
      PengApi restored = new SessionVault(context).load(api.origin);
      assertNotNull(restored);
      assertEquals(api.wallet, restored.wallet);
      assertEquals(api.sessionCookie(), restored.sessionCookie());
      assertNull(new SessionVault(context).load("https://another-server.invalid"));
      new SessionVault(context).clear(api.origin);
      assertNull(new SessionVault(context).load(api.origin));
    } finally {
      first.clear(api.origin);
    }
  }

  @Test
  public void sharedDesktopInterfaceLoadsAndExposesAllPages() throws Exception {
    android.app.Instrumentation instrumentation = InstrumentationRegistry.getInstrumentation();
    Context context = instrumentation.getTargetContext();
    Activity activity =
        instrumentation.startActivitySync(
            new Intent(context, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
    try {
      String result = "";
      for (int attempt = 0; attempt < 60; attempt++) {
        AtomicReference<String> html = new AtomicReference<>();
        CountDownLatch loaded = new CountDownLatch(1);
        instrumentation.runOnMainSync(
            () -> {
              WebView web =
                  (WebView)
                      ((FrameLayout) activity.findViewById(android.R.id.content)).getChildAt(0);
              web.evaluateJavascript(
                  "JSON.stringify({text:document.body.innerText,bridge:!!window.PengMinerAndroid,overflow:document.documentElement.scrollWidth>innerWidth})",
                  value -> {
                    html.set(value);
                    loaded.countDown();
                  });
            });
        assertTrue(loaded.await(5, TimeUnit.SECONDS));
        result = html.get();
        if (result != null && result.contains("Make your compute count")) break;
        Thread.sleep(500);
      }
      assertNotNull(result);
      for (String page :
          new String[] {"Dashboard", "Devices", "Mining", "Wallet", "Settings", "Logs"})
        assertTrue(result, result.contains(page));
      assertFalse(result.contains("Desktop backend unavailable"));
      JSONObject rendered = new JSONObject((String) new org.json.JSONTokener(result).nextValue());
      assertTrue(rendered.getBoolean("bridge"));
      assertFalse("Mobile layout overflows horizontally", rendered.getBoolean("overflow"));
    } finally {
      instrumentation.runOnMainSync(activity::finish);
    }
  }

  private void fixtureTrust() throws Exception {
    String encoded = InstrumentationRegistry.getArguments().getString("fixtureCertificate");
    org.junit.Assume.assumeNotNull(encoded);
    java.security.KeyStore store =
        java.security.KeyStore.getInstance(java.security.KeyStore.getDefaultType());
    store.load(null, null);
    byte[] pem = android.util.Base64.decode(encoded, android.util.Base64.DEFAULT);
    store.setCertificateEntry(
        "fixture",
        java.security.cert.CertificateFactory.getInstance("X.509")
            .generateCertificate(new java.io.ByteArrayInputStream(pem)));
    javax.net.ssl.TrustManagerFactory trust =
        javax.net.ssl.TrustManagerFactory.getInstance(
            javax.net.ssl.TrustManagerFactory.getDefaultAlgorithm());
    trust.init(store);
    javax.net.ssl.SSLContext tls = javax.net.ssl.SSLContext.getInstance("TLS");
    tls.init(null, trust.getTrustManagers(), null);
    javax.net.ssl.HttpsURLConnection.setDefaultSSLSocketFactory(tls.getSocketFactory());
  }

  @Test
  public void nativeMiningAndSavedSession() throws Exception {
    fixtureTrust();
    android.app.Instrumentation instrumentation = InstrumentationRegistry.getInstrumentation();
    Context context = instrumentation.getTargetContext();
    JSONObject settings =
        MinerController.defaults()
            .put("server", InstrumentationRegistry.getArguments().getString("fixtureServer"))
            .put("intensity", 100)
            .put("notifications", true)
            .put("minimizeToTray", true);
    java.nio.file.Files.write(
        new java.io.File(context.getFilesDir(), "settings.json").toPath(),
        settings.toString().getBytes(java.nio.charset.StandardCharsets.UTF_8));
    Activity activity =
        instrumentation.startActivitySync(
            new Intent(context, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
    MinerController controller = MinerController.get(context);
    try {
      controller.command(
          "login",
          new JSONObject()
              .put("wallet", InstrumentationRegistry.getArguments().getString("fixtureWallet"))
              .put("password", "android-test-password"));
      controller.command("start_mining", new JSONObject());
      long deadline = android.os.SystemClock.elapsedRealtime() + 60_000;
      while (controller.snapshot().getLong("accepted") < 2
          && android.os.SystemClock.elapsedRealtime() < deadline) {
        assertTrue(controller.snapshot().toString(), controller.snapshot().isNull("error"));
        Thread.sleep(50);
      }
      JSONObject stopped = (JSONObject) controller.command("stop_mining", new JSONObject());
      assertFalse(stopped.getBoolean("running"));
      assertTrue(stopped.getLong("hashes") > 0);
      assertTrue(stopped.getLong("accepted") >= 2);
      assertEquals(
          stopped.getLong("earnings"),
          stopped.getJSONArray("balances").getJSONObject(0).getLong("amount"));
      assertTrue(stopped.getBoolean("persistent"));
      if (android.os.Build.VERSION.SDK_INT < 33
          || context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
              == android.content.pm.PackageManager.PERMISSION_GRANTED) {
        boolean notified = false;
        for (android.service.notification.StatusBarNotification notification :
            context
                .getSystemService(android.app.NotificationManager.class)
                .getActiveNotifications()) {
          if (notification.getId() == 2) notified = true;
        }
        assertTrue("Enabled reward notifications were not delivered", notified);
      }
      assertFalse(stopped.toString().contains("android-test-password"));
      assertFalse(stopped.toString().contains("peng_session="));
      assertTrue(stopped.isNull("sessionNotice"));
    } finally {
      controller.command("stop_mining", new JSONObject());
      instrumentation.runOnMainSync(activity::finish);
    }
  }

  @Test
  public void restoreAfterProcessRestartAndSignOut() throws Exception {
    fixtureTrust();
    Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
    MinerController controller = MinerController.get(context);
    JSONObject restored = (JSONObject) controller.command("snapshot", new JSONObject());
    assertEquals(
        InstrumentationRegistry.getArguments().getString("fixtureWallet"),
        restored.getString("walletId"));
    assertFalse(restored.getBoolean("running"));
    assertTrue(restored.getBoolean("persistent"));
    assertTrue(restored.getJSONArray("history").length() >= 2);
    PengApi saved =
        new SessionVault(context).load(restored.getJSONObject("settings").getString("server"));
    assertNotNull(saved);
    controller.command("logout", new JSONObject());
    assertNull(new SessionVault(context).load(saved.origin));
    try {
      saved.request("GET", "/wallet/balance", null);
      fail("Server still accepts revoked session");
    } catch (PengApi.Failure expected) {
      assertEquals(401, expected.status);
    }
    controller.command(
        "save_settings", new JSONObject().put("settings", MinerController.defaults()));
  }
}

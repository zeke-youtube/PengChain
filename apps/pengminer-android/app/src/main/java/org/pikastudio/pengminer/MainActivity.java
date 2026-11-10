package org.pikastudio.pengminer;

import android.Manifest;
import android.app.Activity;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.webkit.JavascriptInterface;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import java.io.ByteArrayInputStream;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.function.Consumer;
import org.json.JSONObject;

/** The exact desktop React bundle, served locally with a narrow native bridge. */
public final class MainActivity extends Activity {
  private static final String HOST = "appassets.androidplatform.net";
  private WebView web;
  private MinerController controller;
  private final ExecutorService commands = Executors.newSingleThreadExecutor();
  private volatile boolean destroyed;
  private final Consumer<JSONObject> events = event -> dispatch("pengminer:event", event);

  @android.annotation.SuppressLint(
      "SetJavaScriptEnabled") // Only bundled, intercepted assets can execute; navigation and remote
  // requests are blocked.
  @Override
  public void onCreate(Bundle state) {
    super.onCreate(state);
    controller = MinerController.get(this);
    web = new WebView(this);
    // Android 8 WebView compositor drivers can leave black tiles in fixed/grid layouts.
    if (Build.VERSION.SDK_INT < 28) web.setLayerType(android.view.View.LAYER_TYPE_SOFTWARE, null);
    web.setBackgroundColor(android.graphics.Color.rgb(16, 21, 19));
    if (Build.VERSION.SDK_INT >= 35) {
      web.setOnApplyWindowInsetsListener(
          (view, insets) -> {
            android.graphics.Insets bars =
                insets.getInsets(android.view.WindowInsets.Type.systemBars());
            view.setPadding(bars.left, bars.top, bars.right, bars.bottom);
            return insets;
          });
    }
    WebSettings settings = web.getSettings();
    settings.setJavaScriptEnabled(true);
    settings.setAllowFileAccess(false);
    settings.setAllowContentAccess(false);
    settings.setDomStorageEnabled(false);
    settings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
    settings.setSaveFormData(false);
    settings.setJavaScriptCanOpenWindowsAutomatically(false);
    web.setImportantForAutofill(android.view.View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS);
    web.addJavascriptInterface(new Bridge(), "PengMinerAndroid");
    web.setWebViewClient(
        new WebViewClient() {
          @Override
          public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
            return !local(request.getUrl());
          }

          @Override
          public void onPageStarted(WebView view, String url, android.graphics.Bitmap icon) {
            if (!local(Uri.parse(url))) {
              view.removeJavascriptInterface("PengMinerAndroid");
              view.stopLoading();
            }
          }

          @Override
          public WebResourceResponse shouldInterceptRequest(
              WebView view, WebResourceRequest request) {
            Uri uri = request.getUrl();
            if (!local(uri)) return blocked();
            String path = uri.getPath();
            if (path == null || path.contains("..") || path.indexOf('\\') >= 0) return blocked();
            if ("/".equals(path)) path = "/index.html";
            String type =
                path.endsWith(".js")
                    ? "application/javascript"
                    : path.endsWith(".css")
                        ? "text/css"
                        : path.endsWith(".svg") ? "image/svg+xml" : "text/html";
            try {
              Map<String, String> headers = new HashMap<>();
              headers.put(
                  "Content-Security-Policy",
                  "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src"
                      + " 'self' data:; font-src 'self'; connect-src 'none'; object-src 'none';"
                      + " frame-src 'none'; base-uri 'none'; form-action 'none'");
              headers.put("X-Content-Type-Options", "nosniff");
              return new WebResourceResponse(
                  type, "UTF-8", 200, "OK", headers, getAssets().open("www" + path));
            } catch (java.io.IOException missing) {
              return blocked();
            }
          }
        });
    setContentView(web);
    web.loadUrl("https://" + HOST + "/index.html");
    if (Build.VERSION.SDK_INT >= 33
        && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED) {
      requestPermissions(new String[] {Manifest.permission.POST_NOTIFICATIONS}, 1);
    }
  }

  private static boolean local(Uri uri) {
    return "https".equals(uri.getScheme()) && HOST.equals(uri.getHost());
  }

  private static WebResourceResponse blocked() {
    return new WebResourceResponse(
        "text/plain",
        "UTF-8",
        403,
        "Blocked",
        new HashMap<>(),
        new ByteArrayInputStream(new byte[0]));
  }

  private void dispatch(String type, JSONObject payload) {
    runOnUiThread(
        () -> {
          if (!destroyed)
            web.evaluateJavascript(
                "window.dispatchEvent(new CustomEvent("
                    + JSONObject.quote(type)
                    + ",{detail:"
                    + payload.toString()
                    + "}))",
                null);
        });
  }

  private final class Bridge {
    @JavascriptInterface
    public void request(String id, String name, String serialized) {
      if (id == null
          || !id.matches("[0-9]{1,12}")
          || serialized == null
          || serialized.length() > 65_536) return;
      commands.execute(
          () -> {
            JSONObject reply = new JSONObject();
            try {
              reply
                  .put("id", id)
                  .put("result", controller.command(name, new JSONObject(serialized)));
            } catch (Exception error) {
              controller.fail(error);
              try {
                reply
                    .put("id", id)
                    .put(
                        "error",
                        error instanceof java.io.IOException
                            ? "Could not reach the PengChain server"
                            : java.util.Objects.toString(
                                error.getMessage(), "Native command failed"));
              } catch (Exception ignored) {
              }
            }
            dispatch("pengminer:response", reply);
          });
    }
  }

  @Override
  public void onResume() {
    super.onResume();
    controller.listen(events);
  }

  @Override
  public void onStop() {
    super.onStop();
    try {
      if (!isChangingConfigurations() && !controller.settings().getBoolean("minimizeToTray"))
        controller.requestStop();
    } catch (Exception ignored) {
      controller.requestStop();
    }
  }

  @Override
  public void onDestroy() {
    destroyed = true;
    controller.unlisten(events);
    commands.shutdown();
    web.removeJavascriptInterface("PengMinerAndroid");
    web.destroy();
    super.onDestroy();
  }
}

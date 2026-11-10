package org.pikastudio.pengminer;

import android.content.Context;
import android.content.SharedPreferences;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;
import org.json.JSONObject;

/** Non-exportable Android Keystore key; the saved ciphertext contains no password. */
final class SessionVault {
  private static final String KEY = "pengminer.sessions.v1";
  private final SharedPreferences preferences;

  SessionVault(Context context) {
    preferences = context.getSharedPreferences("encrypted-sessions", Context.MODE_PRIVATE);
  }

  private SecretKey key() throws Exception {
    KeyStore store = KeyStore.getInstance("AndroidKeyStore");
    store.load(null);
    if (store.containsAlias(KEY)) return (SecretKey) store.getKey(KEY, null);
    KeyGenerator generator =
        KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
    generator.init(
        new KeyGenParameterSpec.Builder(
                KEY, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .build());
    return generator.generateKey();
  }

  synchronized void save(PengApi api) throws Exception {
    Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
    cipher.init(Cipher.ENCRYPT_MODE, key());
    byte[] plain =
        new JSONObject()
            .put("server", api.origin)
            .put("wallet", api.wallet)
            .put("cookie", api.sessionCookie())
            .put("remembered", api.remembered)
            .toString()
            .getBytes(StandardCharsets.UTF_8);
    byte[] encrypted;
    try {
      encrypted = cipher.doFinal(plain);
    } finally {
      java.util.Arrays.fill(plain, (byte) 0);
    }
    String value =
        Base64.encodeToString(cipher.getIV(), Base64.NO_WRAP)
            + ":"
            + Base64.encodeToString(encrypted, Base64.NO_WRAP);
    if (!preferences.edit().putString(api.origin, value).commit()) {
      throw new IllegalStateException("Could not save your login securely");
    }
  }

  synchronized PengApi load(String server) throws Exception {
    String value = preferences.getString(server, null);
    if (value == null) return null;
    String[] parts = value.split(":", 2);
    if (parts.length != 2) throw new IllegalStateException("Invalid encrypted session");
    Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
    cipher.init(
        Cipher.DECRYPT_MODE,
        key(),
        new GCMParameterSpec(128, Base64.decode(parts[0], Base64.NO_WRAP)));
    byte[] plain = cipher.doFinal(Base64.decode(parts[1], Base64.NO_WRAP));
    try {
      JSONObject session = new JSONObject(new String(plain, StandardCharsets.UTF_8));
      if (!server.equals(session.getString("server")))
        throw new IllegalStateException("Session origin differs");
      PengApi api = new PengApi(server, session.getString("wallet"));
      api.restoreCookie(session.getString("cookie"));
      api.remembered = session.optBoolean("remembered", false);
      return api;
    } finally {
      java.util.Arrays.fill(plain, (byte) 0);
    }
  }

  synchronized void clear(String server) {
    if (!preferences.edit().remove(server).commit())
      throw new IllegalStateException("Could not clear saved login");
  }
}

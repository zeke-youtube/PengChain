package org.pikastudio.pengminer;

import org.json.JSONObject;
import java.io.InputStream;
import java.net.URI;
import java.net.HttpURLConnection;
import java.nio.charset.StandardCharsets;

/** Per-session origin and cookie. Neither passwords nor cookies are persisted. */
final class PengApi {
    final String origin;
    final String wallet;
    private String cookie;

    PengApi(String server, String wallet) throws Exception {
        URI uri = new URI(server);
        if (!"https".equals(uri.getScheme()) || uri.getHost() == null || uri.getUserInfo() != null
                || uri.getRawQuery() != null || uri.getFragment() != null
                || !(uri.getPath().isEmpty() || "/".equals(uri.getPath()))) {
            throw new IllegalArgumentException("Enter an HTTPS server origin without a path or credentials");
        }
        this.origin = server.endsWith("/") ? server.substring(0, server.length() - 1) : server;
        this.wallet = wallet;
    }

    void login(String password) throws Exception {
        JSONObject body = new JSONObject().put("walletId", wallet).put("password", password);
        request("POST", "/wallet/login", body, true);
        if (cookie == null) throw new IllegalStateException("Server did not issue a session");
    }

    Object request(String method, String path, JSONObject body) throws Exception {
        return request(method, path, body, false);
    }

    private Object request(String method, String path, JSONObject body, boolean login) throws Exception {
        HttpURLConnection connection = (HttpURLConnection) new URI(origin + "/blockchain/api" + path).toURL().openConnection();
        connection.setInstanceFollowRedirects(false);
        connection.setConnectTimeout(10_000);
        connection.setReadTimeout(15_000);
        connection.setRequestMethod(method);
        connection.setRequestProperty("Accept", "application/json");
        if (cookie != null) connection.setRequestProperty("Cookie", cookie);
        try {
            if (body != null) {
                connection.setDoOutput(true);
                connection.setRequestProperty("Content-Type", "application/json");
                try (java.io.OutputStream output = connection.getOutputStream()) {
                    output.write(body.toString().getBytes(StandardCharsets.UTF_8));
                }
            }
            int status = connection.getResponseCode();
            if (status < 200 || status >= 300) {
                // Do not expose server bodies or credentials in status/notifications.
                throw new IllegalStateException("Server returned HTTP " + status + ". Mining stopped; check your balance before restarting.");
            }
            if (login) {
                for (java.util.Map.Entry<String, java.util.List<String>> entry : connection.getHeaderFields().entrySet()) {
                    if ("set-cookie".equalsIgnoreCase(entry.getKey())) {
                        for (String value : entry.getValue()) {
                            if (value.startsWith("peng_session=")) cookie = value.split(";", 2)[0];
                        }
                    }
                }
            }
            try (InputStream input = connection.getInputStream()) {
                ByteArrayOutputStreamLimited bytes = new ByteArrayOutputStreamLimited();
                byte[] buffer = new byte[4096];
                int length;
                while ((length = input.read(buffer)) != -1) bytes.append(buffer, length);
                return new org.json.JSONTokener(bytes.text()).nextValue();
            }
        } finally {
            connection.disconnect();
        }
    }

    private static final class ByteArrayOutputStreamLimited {
        private final java.io.ByteArrayOutputStream bytes = new java.io.ByteArrayOutputStream();
        void append(byte[] buffer, int length) {
            if (bytes.size() + length > 1_048_576) throw new IllegalStateException("Server response too large");
            bytes.write(buffer, 0, length);
        }
        String text() { return new String(bytes.toByteArray(), StandardCharsets.UTF_8); }
    }
}

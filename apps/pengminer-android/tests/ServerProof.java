package org.pikastudio.pengminer;

import java.nio.file.*;
import java.security.*;
import java.security.cert.CertificateFactory;
import java.util.concurrent.atomic.*;
import javax.net.ssl.*;
import org.json.*;

/** Test-only CA trust; the shipped app always uses Android's system TLS trust. */
public final class ServerProof {
    public static void main(String[] args) throws Exception {
        KeyStore store = KeyStore.getInstance(KeyStore.getDefaultType());
        store.load(null, null);
        try (java.io.InputStream input = Files.newInputStream(Path.of(args[1]))) {
            store.setCertificateEntry("fixture", CertificateFactory.getInstance("X.509").generateCertificate(input));
        }
        TrustManagerFactory trust = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());
        trust.init(store);
        SSLContext tls = SSLContext.getInstance("TLS");
        tls.init(null, trust.getTrustManagers(), null);
        HttpsURLConnection.setDefaultSSLSocketFactory(tls.getSocketFactory());
        for (String origin : new String[] {"http://example.com", "https://user:pass@example.com", "https://example.com/path", "https://example.com/?secret=x"}) {
            try { new PengApi(origin, "wallet"); throw new AssertionError("Accepted unsafe origin"); }
            catch (IllegalArgumentException expected) { }
        }
        PengApi api = new PengApi(args[0], args[2]);
        try { api.login("wrong-password"); throw new AssertionError("Bad password accepted"); }
        catch (IllegalStateException expected) { }
        api.login("android-test-password");
        JSONObject job = (JSONObject) api.request("GET", "/mining/job?symbol=PENG", null);
        AtomicLong hashes = new AtomicLong();
        String nonce = Pca10.solve(Pca10.prefix("pengchain-v1", job.getString("jobId"), job.getString("tokenId"),
                job.getString("walletId"), job.getString("challenge"), job.getInt("difficulty")),
                job.getInt("difficulty"), job.getLong("expiresAt"), new AtomicBoolean(), hashes, 100);
        if (nonce == null || hashes.get() == 0) throw new AssertionError("No real proof");
        JSONObject submission = new JSONObject().put("jobId", job.getString("jobId")).put("nonce", nonce);
        JSONObject receipt = (JSONObject) api.request("POST", "/mining/submit", submission);
        if (!receipt.getBoolean("accepted") || receipt.getLong("reward") < 5 || receipt.getLong("reward") > 15) {
            throw new AssertionError("Invalid reward receipt");
        }
        JSONArray balances = (JSONArray) api.request("GET", "/wallet/balance", null);
        if (balances.getJSONObject(0).getLong("amount") != receipt.getLong("reward")) throw new AssertionError("Balance mismatch");
        try { api.request("POST", "/mining/submit", submission); throw new AssertionError("Replay accepted"); }
        catch (IllegalStateException expected) { }
        System.out.println("PASS: HTTPS login, unsafe origins, real PCA-10 proof (" + hashes.get() +
                " hashes), accepted reward, wallet balance, replay rejection");
    }
}

package org.pikastudio.pengminer;

import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.Base64;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;

/** Exact PengChain v1 encoding, including raw challenge bytes and a uint64 nonce. */
public final class Pca10 {
    private Pca10() {}

    public static byte[] prefix(String network, String job, String token, String wallet,
                                String challenge, int difficulty) throws Exception {
        byte[] decoded = Base64.getUrlDecoder().decode(challenge);
        if (decoded.length != 32 || difficulty < 1 || difficulty > 64) {
            throw new IllegalArgumentException("Invalid mining job");
        }
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        DataOutputStream out = new DataOutputStream(bytes);
        for (byte[] field : new byte[][] {new byte[] {1}, utf8(network), utf8(job),
                utf8(token), utf8(wallet), decoded}) {
            out.writeInt(field.length);
            out.write(field);
        }
        return bytes.toByteArray();
    }

    private static byte[] utf8(String value) { return value.getBytes(StandardCharsets.UTF_8); }

    public static byte[] digest(MessageDigest sha, byte[] input) {
        byte[] result = input;
        for (int round = 0; round < 10; round++) result = sha.digest(result);
        return result;
    }

    public static boolean meets(byte[] digest, int difficulty) {
        if (difficulty < 1 || difficulty > 64) return false;
        for (int i = 0; i < difficulty / 2; i++) if (digest[i] != 0) return false;
        return difficulty % 2 == 0 || (digest[difficulty / 2] & 0xf0) == 0;
    }

    public static String solve(byte[] prefix, int difficulty, long expiresAt,
                               AtomicBoolean stop, AtomicLong hashes, int intensity) throws Exception {
        if (intensity != 25 && intensity != 50 && intensity != 100) {
            throw new IllegalArgumentException("Invalid intensity");
        }
        MessageDigest sha = MessageDigest.getInstance("SHA-256");
        byte[] input = java.util.Arrays.copyOf(prefix, prefix.length + 8);
        long nonce = 0;
        while (!stop.get() && System.currentTimeMillis() < expiresAt) {
            long started = System.nanoTime();
            for (int batch = 0; batch < 128; batch++) {
                if (stop.get() || System.currentTimeMillis() >= expiresAt) return null;
                for (int i = 0; i < 8; i++) input[prefix.length + i] = (byte) (nonce >>> (56 - i * 8));
                boolean valid = meets(digest(sha, input), difficulty);
                hashes.incrementAndGet();
                if (valid) return Long.toUnsignedString(nonce);
                if (nonce == -1L) throw new IllegalStateException("Nonce space exhausted");
                nonce++;
            }
            if (intensity < 100) {
                long pause = Math.min(20_000_000L, (System.nanoTime() - started) * (100 / intensity - 1));
                Thread.sleep(pause / 1_000_000L, (int) (pause % 1_000_000L));
            }
        }
        return null;
    }
}

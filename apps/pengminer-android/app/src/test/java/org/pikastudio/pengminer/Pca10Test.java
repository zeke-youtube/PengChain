package org.pikastudio.pengminer;

import static org.junit.Assert.*;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.concurrent.atomic.*;
import org.junit.Test;

public class Pca10Test {
  private byte[] prefix(int difficulty) throws Exception {
    return Pca10.prefix(
        "pengchain-v1",
        "job",
        "PENG",
        "wallet",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        difficulty);
  }

  @Test
  public void matchesRustAndPythonVector() throws Exception {
    AtomicLong hashes = new AtomicLong();
    assertEquals(
        "6306",
        Pca10.solve(
            prefix(3), 3, System.currentTimeMillis() + 60_000, new AtomicBoolean(), hashes, 100));
    assertEquals(6307, hashes.get());
    byte[] digest =
        Pca10.digest(
            MessageDigest.getInstance("SHA-256"), "pengchain".getBytes(StandardCharsets.UTF_8));
    assertEquals(
        "ab2b961b9fa292a39d4a71668ac6449304a60d680615471fd352ea111226bddb",
        java.util.HexFormat.of().formatHex(digest));
  }

  @Test
  public void cancellationAndExpiryDoNoWork() throws Exception {
    AtomicLong hashes = new AtomicLong();
    assertNull(
        Pca10.solve(
            prefix(64),
            64,
            System.currentTimeMillis() + 60_000,
            new AtomicBoolean(true),
            hashes,
            25));
    assertNull(Pca10.solve(prefix(64), 64, 0, new AtomicBoolean(), hashes, 25));
    assertEquals(0, hashes.get());
  }

  @Test
  public void stopsAnActiveHardJob() throws Exception {
    AtomicBoolean stop = new AtomicBoolean();
    AtomicLong hashes = new AtomicLong();
    java.util.concurrent.ExecutorService worker =
        java.util.concurrent.Executors.newSingleThreadExecutor();
    try {
      java.util.concurrent.Future<String> result =
          worker.submit(
              () ->
                  Pca10.solve(
                      prefix(64), 64, System.currentTimeMillis() + 60_000, stop, hashes, 25));
      long deadline = System.nanoTime() + 2_000_000_000L;
      while (hashes.get() == 0 && System.nanoTime() < deadline) Thread.sleep(1);
      assertTrue(hashes.get() > 0);
      stop.set(true);
      assertNull(result.get(1, java.util.concurrent.TimeUnit.SECONDS));
    } finally {
      stop.set(true);
      worker.shutdownNow();
    }
  }

  @Test
  public void parallelWorkersProduceARealProof() throws Exception {
    AtomicLong hashes = new AtomicLong();
    byte[] prefix = prefix(3);
    String nonce =
        Pca10.solveParallel(
            prefix,
            3,
            System.currentTimeMillis() + 60_000,
            new AtomicBoolean(),
            hashes,
            Math.min(2, Runtime.getRuntime().availableProcessors()),
            100);
    assertNotNull(nonce);
    byte[] input = java.util.Arrays.copyOf(prefix, prefix.length + 8);
    long value = Long.parseUnsignedLong(nonce);
    for (int i = 0; i < 8; i++) input[prefix.length + i] = (byte) (value >>> (56 - i * 8));
    assertTrue(Pca10.meets(Pca10.digest(MessageDigest.getInstance("SHA-256"), input), 3));
    assertTrue(hashes.get() > 0);
  }

  @Test
  public void difficultyUsesHexNibbles() {
    byte[] digest = new byte[32];
    digest[1] = 15;
    assertTrue(Pca10.meets(digest, 3));
    assertFalse(Pca10.meets(digest, 4));
    assertFalse(Pca10.meets(digest, 65));
    assertTrue(Pca10.meets(new byte[32], 64));
  }

  @Test(expected = IllegalArgumentException.class)
  public void rejectsMalformedChallenge() throws Exception {
    Pca10.prefix("n", "j", "t", "w", "bad", 1);
  }
}

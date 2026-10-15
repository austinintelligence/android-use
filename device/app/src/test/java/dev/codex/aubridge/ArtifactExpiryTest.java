package dev.codex.aubridge;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public class ArtifactExpiryTest {
    private static final long HOUR = 60L * 60 * 1000;

    @Test
    public void capturesExpireAfterTwentyFourHours() {
        long now = 100 * HOUR;
        assertFalse(Capture.expired(now - 23 * HOUR, now));
        assertFalse(Capture.expired(now - 24 * HOUR, now));
        assertTrue(Capture.expired(now - 24 * HOUR - 1, now));
    }
}

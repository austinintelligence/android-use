package dev.codex.aubridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public class SafetyTest {
    @Test
    public void allZeroAudioIsSilent() {
        assertTrue(Safety.silent(new byte[64], 64));
    }

    @Test
    public void anySampleMakesAudioSound() {
        byte[] pcm = new byte[64];
        pcm[63] = 1;
        assertFalse(Safety.silent(pcm, 64));
    }

    @Test
    public void onlyTheBytesReadAreChecked() {
        byte[] pcm = new byte[64];
        pcm[40] = 7;
        assertTrue(Safety.silent(pcm, 40));
    }

    @Test
    public void cameraOpenedAfterTimeoutIsClosedImmediately() {
        Safety.Abandon late = new Safety.Abandon();
        StringBuilder log = new StringBuilder();
        late.abandon();
        late.offer(() -> log.append("kept"), () -> log.append("closed"));
        assertEquals("closed", log.toString());
    }

    @Test
    public void cameraOpenedInTimeIsKept() {
        Safety.Abandon late = new Safety.Abandon();
        StringBuilder log = new StringBuilder();
        late.offer(() -> log.append("kept"), () -> log.append("closed"));
        assertEquals("kept", log.toString());
    }

    @Test
    public void readEndsOnceEveryProviderIsDisabled() {
        Safety.Providers live = new Safety.Providers(2);
        assertFalse(live.lost());
        assertTrue(live.lost());
        assertTrue(live.none());
    }

    @Test
    public void noProvidersMeansNothingToWaitFor() {
        assertTrue(new Safety.Providers(0).none());
    }
}

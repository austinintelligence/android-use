package dev.codex.aubridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.junit.Test;

public class RecordingTest {
    @Test
    public void displayBeforeCallbackIsRejected() {
        Recording r = new Recording();
        r.reach(Recording.Step.SERVICE_PROMOTED);
        r.reach(Recording.Step.PROJECTION_OBTAINED);
        try {
            r.reach(Recording.Step.DISPLAY_CREATED);
            fail("display created before the projection callback was registered");
        } catch (IllegalStateException expected) {
        }
    }

    @Test
    public void projectionBeforeServicePromotionIsRejected() {
        try {
            new Recording().reach(Recording.Step.PROJECTION_OBTAINED);
            fail("projection obtained before the foreground service was promoted");
        } catch (IllegalStateException expected) {
        }
    }

    @Test
    public void fullOrderIsAccepted() {
        Recording r = new Recording();
        for (Recording.Step s : Recording.Step.values()) r.reach(s);
    }

    @Test
    public void tokenIsClearedAfterUseOnAndroid14() {
        Recording.Token<String> token = new Recording.Token<>();
        token.set("grant");
        assertEquals("grant", token.take(34));
        assertFalse(token.present());
        assertNull(token.take(34));
    }

    @Test
    public void tokenSurvivesUseOnOlderAndroid() {
        Recording.Token<String> token = new Recording.Token<>();
        token.set("grant");
        assertEquals("grant", token.take(33));
        assertTrue(token.present());
        assertEquals("grant", token.take(29));
    }
}

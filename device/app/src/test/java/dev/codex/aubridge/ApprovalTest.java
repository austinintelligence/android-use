package dev.codex.aubridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Before;
import org.junit.Test;

public class ApprovalTest {
    @Before
    public void reset() {
        Approval.clear();
    }

    @Test
    public void aPromptIsPendingUntilAPersonAnswers() {
        assertEquals("none", Approval.current(0));
        assertTrue(Approval.start("Pay $25?", "tap \"Pay $25\"", 60_000, 1_000));
        assertEquals("pending", Approval.current(2_000));
        Approval.answer(true, 3_000);
        assertEquals("allow", Approval.current(3_000));
        Approval.clear();
        assertEquals("none", Approval.current(3_000));
    }

    @Test
    public void silenceTimesOutAndALateAnswerChangesNothing() {
        Approval.start("Send?", "tap \"Send\"", 10_000, 0);
        assertEquals("pending", Approval.current(9_999));
        assertEquals("timeout", Approval.current(10_000));
        Approval.answer(true, 10_001);
        assertEquals("timeout", Approval.current(10_001));
    }

    @Test
    public void onlyOnePromptWaitsAtATimeAndTheTimeoutIsBounded() {
        assertTrue(Approval.start("a", "x", 1, 0));
        assertFalse(Approval.start("b", "y", 60_000, 1));
        assertEquals("a", Approval.text());
        assertEquals("pending", Approval.current(Approval.MIN_MS - 1));
        Approval.clear();
        Approval.start("c", "z", Long.MAX_VALUE / 2, 0);
        assertEquals("pending", Approval.current(Approval.MAX_MS - 1));
        assertEquals("timeout", Approval.current(Approval.MAX_MS));
    }

    @Test
    public void denyIsRecordedAndTextIsClipped() {
        StringBuilder long400 = new StringBuilder();
        for (int i = 0; i < 400; i++) long400.append('x');
        Approval.start(long400.toString(), long400.toString(), 60_000, 0);
        assertEquals(300, Approval.text().length());
        assertEquals(300, Approval.detail().length());
        Approval.answer(false, 1);
        assertEquals("deny", Approval.current(1));
        assertFalse(Approval.waiting(1));
    }
}

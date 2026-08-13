package dev.codex.aubridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotEquals;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import org.junit.Test;

public final class SessionAuthTest {
    @Test
    public void issuesUniqueShortLivedCredentials() throws Exception {
        SessionAuth auth = new SessionAuth();
        String first = auth.issueSession();
        String second = auth.issueSession();
        assertNotEquals(first, second);
        assertTrue(first.length() >= 32);
        auth.authenticate(first, "nonce-00000001");
        assertEquals(2, auth.sessionCountForTests());
    }

    @Test
    public void rejectsWrongOrPersistentToken() throws Exception {
        SessionAuth auth = new SessionAuth();
        String issued = auth.issueSession();
        try {
            auth.authenticate("secret-token", "nonce-00000001");
            fail("unknown token must be rejected");
        } catch (SessionAuth.AuthError error) {
            assertEquals("E_AUTH", error.code);
        }
        auth.authenticate(issued, "nonce-00000001");
    }

    @Test
    public void rejectsShortAndReplayedNonce() throws Exception {
        SessionAuth auth = new SessionAuth();
        String issued = auth.issueSession();
        try {
            auth.authenticate(issued, "short");
            fail("short nonce must be rejected");
        } catch (SessionAuth.AuthError error) {
            assertEquals("E_AUTH", error.code);
        }
        auth.authenticate(issued, "nonce-00000001");
        try {
            auth.authenticate(issued, "nonce-00000001");
            fail("replayed nonce must be rejected");
        } catch (SessionAuth.AuthError error) {
            assertEquals("E_AUTH", error.code);
        }
    }

    @Test
    public void rejectsOversizedNonceBeforeReplayStateCanGrow() throws Exception {
        SessionAuth auth = new SessionAuth();
        String issued = auth.issueSession();
        try {
            auth.authenticate(issued, "n".repeat(129));
            fail("oversized nonce must be rejected");
        } catch (SessionAuth.AuthError error) {
            assertEquals("E_AUTH", error.code);
        }
    }

    @Test
    public void boundsSessionTable() {
        SessionAuth auth = new SessionAuth();
        for (int i = 0; i < SessionAuth.MAX_SESSIONS + 8; i++) {
            auth.issueSession();
        }
        assertEquals(SessionAuth.MAX_SESSIONS, auth.sessionCountForTests());
    }

    @Test
    public void expiresAndPurgesAtTheSessionTtlBoundary() throws Exception {
        long[] now = {0L};
        SessionAuth auth = new SessionAuth(() -> now[0]);
        String issued = auth.issueSession();
        auth.authenticate(issued, "nonce-00000001");
        now[0] = SessionAuth.SESSION_TTL_MS;
        try {
            auth.authenticate(issued, "nonce-00000002");
            fail("session must expire at its exact TTL boundary");
        } catch (SessionAuth.AuthError error) {
            assertEquals("E_AUTH", error.code);
        }
        assertEquals(0, auth.sessionCountForTests());
    }

    @Test
    public void nonceBudgetNeverEvictsReplayHistory() throws Exception {
        SessionAuth auth = new SessionAuth();
        String issued = auth.issueSession();
        auth.authenticate(issued, "nonce-00000000");
        for (int i = 1; i < SessionAuth.MAX_NONCES_PER_SESSION; i++) {
            auth.authenticate(issued, String.format("nonce-%08d", i));
        }
        try {
            auth.authenticate(issued, "nonce-00000000");
            fail("an old nonce must remain rejected after the budget is full");
        } catch (SessionAuth.AuthError error) {
            assertEquals("E_AUTH", error.code);
        }
        try {
            auth.authenticate(issued, "nonce-budget-new");
            fail("a full session must require fresh bootstrap");
        } catch (SessionAuth.AuthError error) {
            assertEquals("E_AUTH", error.code);
        }
    }

    @Test
    public void requestSequenceIsMonotonicPerConnection() throws Exception {
        BridgeServer.RequestSequence sequence = new BridgeServer.RequestSequence();
        sequence.accept(1L);
        assertEquals(2L, sequence.expected());
        sequence.accept(2L);
        assertEquals(3L, sequence.expected());
        try {
            sequence.accept(2L);
            fail("replayed sequence must be rejected");
        } catch (BridgeServer.BridgeError error) {
            assertEquals("E_AUTH", error.code);
        }
        assertEquals(3L, sequence.expected());
    }
}

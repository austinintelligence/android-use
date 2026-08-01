package dev.codex.aubridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.fail;

import org.junit.Test;

public final class SessionAuthTest {
    @Test
    public void acceptsUniqueNonceWithCorrectToken() throws Exception {
        SessionAuth auth = new SessionAuth("secret-token");
        auth.authenticate("secret-token", "nonce-00000001");
    }

    @Test
    public void rejectsWrongToken() throws Exception {
        try {
            new SessionAuth("secret-token").authenticate("wrong-token", "nonce-00000001");
            fail("wrong token must be rejected");
        } catch (SessionAuth.AuthError error) {
            assertEquals("E_AUTH", error.code);
        }
    }

    @Test
    public void rejectsShortAndReplayedNonce() throws Exception {
        SessionAuth auth = new SessionAuth("secret-token");
        try {
            auth.authenticate("secret-token", "short");
            fail("short nonce must be rejected");
        } catch (SessionAuth.AuthError error) {
            assertEquals("E_AUTH", error.code);
        }
        auth.authenticate("secret-token", "nonce-00000001");
        try {
            auth.authenticate("secret-token", "nonce-00000001");
            fail("replayed nonce must be rejected");
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

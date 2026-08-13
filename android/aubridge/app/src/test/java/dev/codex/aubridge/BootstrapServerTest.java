package dev.codex.aubridge;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public final class BootstrapServerTest {
    @Test
    public void onlyRootAndAdbShellCanBootstrap() {
        assertTrue(BootstrapServer.isAuthorizedUid(0));
        assertTrue(BootstrapServer.isAuthorizedUid(2000));
        assertFalse(BootstrapServer.isAuthorizedUid(1000));
        assertFalse(BootstrapServer.isAuthorizedUid(10000));
    }

    @Test
    public void bootstrapNonceIsBounded() {
        assertFalse(BootstrapServer.isValidNonce("short"));
        assertTrue(BootstrapServer.isValidNonce("nonce-00000001"));
        assertFalse(BootstrapServer.isValidNonce("n".repeat(129)));
    }
}

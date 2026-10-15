package dev.codex.aubridge;

import java.security.GeneralSecurityException;
import java.security.SecureRandom;

import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;

/**
 * The per-process secret that proves this process is the helper. Only the ADB shell user can read
 * it ({@link KeyProvider}); the host answers a random challenge with it before trusting the socket.
 */
final class HelperKey {
    static final int SHELL_UID = 2000;
    static final String CONTEXT = "android-use-helper-v1";
    private static final byte[] KEY = random(32);

    private HelperKey() {}

    static byte[] key() {
        return KEY.clone();
    }

    static boolean mayRead(int callingUid) {
        return callingUid == SHELL_UID;
    }

    /** HMAC-SHA256(key, challenge || CONTEXT): what the host expects from the real helper. */
    static byte[] respond(byte[] key, byte[] challenge) {
        try {
            Mac mac = Mac.getInstance("HmacSHA256");
            mac.init(new SecretKeySpec(key, "HmacSHA256"));
            mac.update(challenge);
            return mac.doFinal(CONTEXT.getBytes(java.nio.charset.StandardCharsets.UTF_8));
        } catch (GeneralSecurityException e) {
            throw new IllegalStateException(e);
        }
    }

    private static byte[] random(int n) {
        byte[] b = new byte[n];
        new SecureRandom().nextBytes(b);
        return b;
    }
}

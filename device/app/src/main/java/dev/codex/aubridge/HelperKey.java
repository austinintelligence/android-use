package dev.codex.aubridge;

import java.nio.charset.StandardCharsets;
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
    static final String BOOTSTRAP_CONTEXT = "android-use-helper-v1";
    static final String COMMAND_CONTEXT = "android-use-command-v1";
    private static final byte[] KEY = random(32);

    private HelperKey() {}

    static byte[] key() {
        return KEY.clone();
    }

    static boolean mayRead(int callingUid) {
        return callingUid == SHELL_UID;
    }

    /** HMAC-SHA256(key, challenge || context): what the host expects from the real helper. */
    static byte[] respond(byte[] key, byte[] challenge) {
        return hmac(key, challenge, BOOTSTRAP_CONTEXT);
    }

    /** The command socket's answer, bound to the token so it cannot be replayed from bootstrap. */
    static byte[] respondCommand(byte[] key, byte[] challenge, String token) {
        byte[] message = new byte[challenge.length + token.length()];
        System.arraycopy(challenge, 0, message, 0, challenge.length);
        System.arraycopy(
                token.getBytes(StandardCharsets.UTF_8),
                0,
                message,
                challenge.length,
                token.length());
        return hmac(key, message, COMMAND_CONTEXT);
    }

    private static byte[] hmac(byte[] key, byte[] message, String context) {
        try {
            Mac mac = Mac.getInstance("HmacSHA256");
            mac.init(new SecretKeySpec(key, "HmacSHA256"));
            mac.update(message);
            return mac.doFinal(context.getBytes(StandardCharsets.UTF_8));
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
